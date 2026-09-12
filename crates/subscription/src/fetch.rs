use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use futures_util::StreamExt;
use thiserror::Error;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const USER_AGENT: &str = concat!("v2ray-rs/", env!("CARGO_PKG_VERSION"));
const MAX_SUBSCRIPTION_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
const MAX_ERROR_BODY_SIZE: usize = 4096; // 4 KiB
#[derive(Debug, Error)]
pub enum FetchError {
    #[error("network error: {0}")]
    NetworkError(String),
    #[error("HTTP {status}: {}", body.replace(['\r', '\n'], " "))]
    HttpError { status: u16, body: String },
    #[error("file error: {0}")]
    FileError(#[from] std::io::Error),
    #[error("request timed out")]
    Timeout,
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("request construction failed: {0}")]
    RequestBuildError(String),
}

impl FetchError {
    pub(crate) fn is_transient(&self) -> bool {
        match self {
            Self::Timeout => true,
            Self::NetworkError(_) => true,
            Self::HttpError { status, .. } => matches!(status, 408 | 429 | 500..=599),
            Self::InvalidUrl(_) => false,
            Self::RequestBuildError(_) => false,
            Self::FileError(_) => false,
        }
    }
}
pub async fn fetch_with_client(client: &reqwest::Client, url: &str) -> Result<String, FetchError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| FetchError::InvalidUrl(e.to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(FetchError::InvalidUrl(format!(
            "unsupported scheme: {} (only http/https)",
            parsed.scheme()
        )));
    }
    if parsed.scheme() == "http" {
        log::warn!("fetching subscription over plaintext HTTP — credentials may be exposed");
    }

    let response = client.get(parsed).send().await.map_err(|e| {
        if e.is_timeout() {
            FetchError::Timeout
        } else if e.is_builder() {
            FetchError::RequestBuildError(e.without_url().to_string())
        } else {
            FetchError::NetworkError(e.without_url().to_string())
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        let body = read_error_body(response).await;
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
        let chunk = chunk.map_err(|e| FetchError::NetworkError(e.without_url().to_string()))?;
        data.extend_from_slice(&chunk);
        if data.len() as u64 > MAX_SUBSCRIPTION_SIZE {
            return Err(FetchError::NetworkError(format!(
                "response too large: > {MAX_SUBSCRIPTION_SIZE} bytes"
            )));
        }
    }

    String::from_utf8(data).map_err(|e| FetchError::NetworkError(e.to_string()))
}

async fn read_error_body(mut response: reqwest::Response) -> String {
    let mut body = Vec::new();
    while body.len() < MAX_ERROR_BODY_SIZE {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let take = (MAX_ERROR_BODY_SIZE - body.len()).min(chunk.len());
                body.extend_from_slice(&chunk[..take]);
            }
            Ok(None) | Err(_) => break,
        }
    }
    String::from_utf8_lossy(&body).into_owned()
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
    use std::sync::Once;

    fn test_client() -> reqwest::Client {
        static RUSTLS_PROVIDER: Once = Once::new();
        RUSTLS_PROVIDER.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
        reqwest::Client::new()
    }

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

    #[test]
    fn is_transient_timeout() {
        assert!(FetchError::Timeout.is_transient());
    }

    #[test]
    fn is_transient_network_error() {
        assert!(FetchError::NetworkError("e".into()).is_transient());
    }

    #[test]
    fn is_transient_invalid_url() {
        assert!(!FetchError::InvalidUrl("bad".into()).is_transient());
    }

    #[test]
    fn is_transient_request_build_error() {
        assert!(!FetchError::RequestBuildError("bad".into()).is_transient());
    }

    #[test]
    fn is_transient_file_error() {
        assert!(!FetchError::FileError(std::io::Error::other("e")).is_transient());
    }

    #[test]
    fn is_transient_http_408() {
        assert!(
            FetchError::HttpError {
                status: 408,
                body: "".into()
            }
            .is_transient()
        );
    }

    #[test]
    fn is_transient_http_429() {
        assert!(
            FetchError::HttpError {
                status: 429,
                body: "".into()
            }
            .is_transient()
        );
    }

    #[test]
    fn is_transient_http_500() {
        assert!(
            FetchError::HttpError {
                status: 500,
                body: "".into()
            }
            .is_transient()
        );
    }

    #[test]
    fn is_transient_http_503() {
        assert!(
            FetchError::HttpError {
                status: 503,
                body: "".into()
            }
            .is_transient()
        );
    }

    #[test]
    fn is_transient_http_404() {
        assert!(
            !FetchError::HttpError {
                status: 404,
                body: "".into()
            }
            .is_transient()
        );
    }

    #[test]
    fn is_transient_http_401() {
        assert!(
            !FetchError::HttpError {
                status: 401,
                body: "".into()
            }
            .is_transient()
        );
    }

    #[test]
    fn invalid_url_display() {
        let err = FetchError::InvalidUrl("not-a-uri".into());
        let msg = err.to_string();
        assert!(
            msg.contains("invalid URL"),
            "expected 'invalid URL' in display, got: {msg}"
        );
    }
    #[tokio::test]
    async fn fetch_with_client_rejects_empty_host() {
        let client = test_client();
        let result = fetch_with_client(&client, "https://").await;
        assert!(
            matches!(result, Err(FetchError::InvalidUrl(_))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn fetch_with_client_rejects_unsupported_scheme() {
        let client = test_client();
        let result = fetch_with_client(&client, "ftp://example.com").await;
        assert!(
            matches!(result, Err(FetchError::InvalidUrl(_))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn fetch_error_display_omits_subscription_url() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/sub");
        // Drop the listener without accepting so the connect refuses
        // immediately with a deterministic "connection refused" instead
        // of stalling on a half-closed accepted socket.
        drop(listener);

        let _ = test_client();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(500))
            .build()
            .unwrap();
        let result = fetch_with_client(&client, &url).await;
        let err = result.expect_err("connecting to a closed port must fail");
        let msg = err.to_string();
        assert!(
            !msg.contains(&addr.to_string()),
            "network error must not embed the subscription URL: {msg}"
        );
    }
    #[test]
    fn http_error_display_sanitizes_newlines() {
        let err = FetchError::HttpError {
            status: 500,
            body: "boom\r\nline2\nline3".into(),
        };
        let msg = err.to_string();
        assert!(
            !msg.contains('\n') && !msg.contains('\r'),
            "display must not embed raw newlines: {msg:?}"
        );
        assert!(msg.starts_with("HTTP 500: "), "{msg:?}");
        for piece in ["boom", "line2", "line3"] {
            assert!(msg.contains(piece), "{msg:?}");
        }
    }

    #[tokio::test]
    async fn fetch_error_body_is_capped_at_4096_bytes() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::task::spawn_blocking(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf); // request line + headers
            let body = "x".repeat(64 * 1024);
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            // The client stops reading after the cap and may close the
            // connection mid-write; a refused write is fine.
            let _ = sock.write_all(response.as_bytes());
            let _ = sock.flush();
        });

        let _ = test_client();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(500))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let result = fetch_with_client(&client, &format!("http://{addr}/sub")).await;
        let err = result.expect_err("HTTP 500 must fail");
        server.await.unwrap();
        let FetchError::HttpError { status, body } = err else {
            panic!("expected HttpError, got: {err:?}")
        };
        assert_eq!(status, 500);
        assert_eq!(body.len(), MAX_ERROR_BODY_SIZE);
        assert!(body.chars().all(|c| c == 'x'));
    }
}
