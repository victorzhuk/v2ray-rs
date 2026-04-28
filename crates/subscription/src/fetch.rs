use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use futures_util::StreamExt;
use thiserror::Error;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const USER_AGENT: &str = concat!("v2ray-rs/", env!("CARGO_PKG_VERSION"));
const MAX_SUBSCRIPTION_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("network error: {0}")]
    NetworkError(String),
    #[error("HTTP {status}: {body}")]
    HttpError { status: u16, body: String },
    #[error("file error: {0}")]
    FileError(#[from] std::io::Error),
    #[error("request timed out")]
    Timeout,
}

pub async fn fetch_with_client(client: &reqwest::Client, url: &str) -> Result<String, FetchError> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(FetchError::NetworkError(
            "only http:// and https:// URLs are supported".into(),
        ));
    }

    if url.starts_with("http://") {
        log::warn!("fetching subscription over plaintext HTTP — credentials may be exposed");
    }

    let response = client.get(url).send().await.map_err(|e| {
        if e.is_timeout() {
            FetchError::Timeout
        } else {
            FetchError::NetworkError(e.to_string())
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(FetchError::HttpError {
            status: status.as_u16(),
            body,
        });
    }

    if let Some(len) = response.content_length()
        && len > MAX_SUBSCRIPTION_SIZE
    {
        return Err(FetchError::NetworkError(format!(
            "response too large: {len} bytes (max {MAX_SUBSCRIPTION_SIZE})"
        )));
    }

    let mut data = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| FetchError::NetworkError(e.to_string()))?;
        data.extend_from_slice(&chunk);
        if data.len() as u64 > MAX_SUBSCRIPTION_SIZE {
            return Err(FetchError::NetworkError(format!(
                "response too large: > {MAX_SUBSCRIPTION_SIZE} bytes"
            )));
        }
    }

    String::from_utf8(data).map_err(|e| FetchError::NetworkError(e.to_string()))
}

pub fn fetch_from_file(path: &str) -> Result<String, FetchError> {
    std::fs::read_to_string(path).map_err(FetchError::FileError)
}

pub fn decode_subscription_content(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();

    let decoded = STANDARD
        .decode(trimmed)
        .or_else(|_| URL_SAFE_NO_PAD.decode(trimmed));

    let text = match decoded {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => trimmed.to_owned(),
    };

    text.lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_base64_content() {
        let uris = "vmess://example1\nvless://example2\nss://example3";
        let encoded = STANDARD.encode(uris);

        let result = decode_subscription_content(&encoded);

        assert_eq!(
            result,
            vec!["vmess://example1", "vless://example2", "ss://example3"]
        );
    }

    #[test]
    fn test_decode_plain_content() {
        let plain = "vmess://example1\nvless://example2\nss://example3";

        let result = decode_subscription_content(plain);

        assert_eq!(
            result,
            vec!["vmess://example1", "vless://example2", "ss://example3"]
        );
    }

    #[test]
    fn test_decode_filters_empty_lines() {
        let input = "vmess://a\n\n\nvless://b\n  \nss://c\n";
        let encoded = STANDARD.encode(input);

        let result = decode_subscription_content(&encoded);

        assert_eq!(result, vec!["vmess://a", "vless://b", "ss://c"]);

        let plain_result = decode_subscription_content(input);
        assert_eq!(plain_result, vec!["vmess://a", "vless://b", "ss://c"]);
    }

    #[test]
    fn test_fetch_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("subs.txt");
        let content = "vmess://test1\nvless://test2";
        std::fs::write(&file_path, content).unwrap();

        let result = fetch_from_file(file_path.to_str().unwrap()).unwrap();

        assert_eq!(result, content);
    }

    #[test]
    fn test_fetch_decode_parse_integration() {
        use crate::parser::parse_subscription_uris;
        use v2ray_rs_core::models::ProxyNode;

        let vmess_json =
            r#"{"add":"vmess.example.com","port":"443","id":"vmess-uuid","ps":"VMess Node"}"#;
        let vmess_uri = format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(vmess_json)
        );

        let ss_userinfo =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("aes-256-gcm:secret");
        let ss_uri = format!("ss://{}@ss.example.com:8388#SS%20Node", ss_userinfo);

        let content = format!(
            "{}\n{}\nvless://uuid@vless.example.com:443#VLESS%20Node\ntrojan://pass@trojan.example.com:443#Trojan%20Node",
            vmess_uri, ss_uri
        );

        let encoded_content = STANDARD.encode(&content);

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("subscription.txt");
        std::fs::write(&file_path, encoded_content).unwrap();

        let raw = fetch_from_file(file_path.to_str().unwrap()).unwrap();
        let uris = decode_subscription_content(&raw);
        let import_result = parse_subscription_uris(&uris);

        assert_eq!(import_result.nodes.len(), 4);
        assert_eq!(import_result.errors.len(), 0);

        let protocols: Vec<_> = import_result
            .nodes
            .iter()
            .map(|n| match &n.node {
                ProxyNode::Vless(_) => "vless",
                ProxyNode::Vmess(_) => "vmess",
                ProxyNode::Shadowsocks(_) => "ss",
                ProxyNode::Trojan(_) => "trojan",
            })
            .collect();

        assert!(protocols.contains(&"vless"));
        assert!(protocols.contains(&"vmess"));
        assert!(protocols.contains(&"ss"));
        assert!(protocols.contains(&"trojan"));

        assert!(import_result.nodes.iter().all(|n| n.enabled));
    }
}
