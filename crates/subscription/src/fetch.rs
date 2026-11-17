use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use futures::StreamExt;
use thiserror::Error;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const USER_AGENT: &str = concat!("v2ray-rs/", env!("CARGO_PKG_VERSION"));
pub(crate) const REDIRECT_CAP: usize = 10;

/// Decides whether one redirect hop may be followed. Refuses a scheme
/// downgrade from `https` to `http`, refuses any non-HTTP(S) scheme, private or
/// local redirect targets, and enforces the hop cap.
pub(crate) fn redirect_allowed(
    scheme_from: Option<&str>,
    next: &reqwest::Url,
    hops: usize,
    cap: usize,
) -> bool {
    if hops >= cap || !redirect_target_is_public(next) {
        return false;
    }
    match next.scheme() {
        "https" => true,
        "http" => !matches!(scheme_from, Some("https")),
        _ => false,
    }
}

/// Rejects redirect targets that would reach the local host or a private
/// network: loopback, link-local, RFC 1918, unique-local, unspecified, and
/// `localhost` names. DNS names are not resolved here; public names stay
/// allowed and private literals cannot masquerade as one.
fn redirect_target_is_public(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = literal.parse::<std::net::IpAddr>() {
        return !ip_is_private_or_local(ip);
    }
    let host = host.to_ascii_lowercase();
    host != "localhost" && !host.ends_with(".localhost")
}

fn ip_is_private_or_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return ip_is_private_or_local(std::net::IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unicast_link_local()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

/// Builds a redirect policy refusing plaintext downgrades, non-HTTP(S)
/// schemes, and private or local targets. Refusals surface as fetch errors
/// naming only the host, never the credential-bearing URL.
pub(crate) fn redirect_policy(cap: usize) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        let hops = attempt.previous().len();
        let scheme_from = attempt.previous().last().map(|url| url.scheme());
        if redirect_allowed(scheme_from, attempt.url(), hops, cap) {
            attempt.follow()
        } else {
            let host = attempt.url().host_str().unwrap_or("unknown host");
            let reason = if hops >= cap {
                format!("too many redirects (host {host})")
            } else if !redirect_target_is_public(attempt.url()) {
                format!("refusing redirect to a private or local target (host {host})")
            } else {
                format!("refusing insecure or unsupported redirect for host {host}")
            };
            attempt.error(reason)
        }
    })
}

const MAX_SUBSCRIPTION_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
#[derive(Debug, Error)]
pub enum FetchError {
    #[error("network error: {0}")]
    NetworkError(String),
    #[error("HTTP {status}: subscription request failed")]
    HttpError { status: u16 },
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
            Self::HttpError { status } => matches!(status, 408 | 429 | 500..=599),
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
        return Err(FetchError::HttpError {
            status: status.as_u16(),
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

pub fn fetch_from_file(path: &str) -> Result<String, FetchError> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(FetchError::FileError(std::io::Error::other(
            "subscription source must be a regular file",
        )));
    }
    let mut data = String::new();
    file.take(MAX_SUBSCRIPTION_SIZE + 1)
        .read_to_string(&mut data)?;
    if data.len() as u64 > MAX_SUBSCRIPTION_SIZE {
        return Err(FetchError::FileError(std::io::Error::other(
            "subscription file exceeds 10 MiB",
        )));
    }
    Ok(data)
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
    fn local_import_refuses_oversized_and_special_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_SUBSCRIPTION_SIZE + 1).unwrap();
        assert!(fetch_from_file(path.to_str().unwrap()).is_err());
        assert!(fetch_from_file("/dev/zero").is_err());
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
        assert!(FetchError::HttpError { status: 408 }.is_transient());
    }

    #[test]
    fn is_transient_http_429() {
        assert!(FetchError::HttpError { status: 429 }.is_transient());
    }

    #[test]
    fn is_transient_http_500() {
        assert!(FetchError::HttpError { status: 500 }.is_transient());
    }

    #[test]
    fn is_transient_http_503() {
        assert!(FetchError::HttpError { status: 503 }.is_transient());
    }

    #[test]
    fn is_transient_http_404() {
        assert!(!FetchError::HttpError { status: 404 }.is_transient());
    }

    #[test]
    fn is_transient_http_401() {
        assert!(!FetchError::HttpError { status: 401 }.is_transient());
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
    #[tokio::test]
    async fn http_error_display_omits_remote_body() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::task::spawn_blocking(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf);
            let body = "boom\r\nline2";
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
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
        let msg = err.to_string();
        assert_eq!(msg, "HTTP 500: subscription request failed");
        assert!(
            !msg.contains("boom"),
            "display must not embed the remote body: {msg:?}"
        );
        assert!(
            !msg.contains('\n') && !msg.contains('\r'),
            "display must not embed raw newlines: {msg:?}"
        );
    }

    #[test]
    fn https_to_https_allowed_within_cap() {
        let next = reqwest::Url::parse("https://example.com/sub").unwrap();
        assert!(redirect_allowed(Some("https"), &next, 0, 5));
        assert!(redirect_allowed(Some("https"), &next, 4, 5));
    }

    #[test]
    fn https_to_http_downgrade_refused() {
        let next = reqwest::Url::parse("http://example.com/sub").unwrap();
        assert!(!redirect_allowed(Some("https"), &next, 0, 5));
    }

    #[test]
    fn http_to_http_allowed_within_cap() {
        let next = reqwest::Url::parse("http://example.com/sub").unwrap();
        assert!(redirect_allowed(Some("http"), &next, 0, 5));
        assert!(redirect_allowed(None, &next, 0, 5));
    }

    #[test]
    fn exhausted_cap_refused() {
        let https = reqwest::Url::parse("https://example.com/sub").unwrap();
        let http = reqwest::Url::parse("http://example.com/sub").unwrap();
        assert!(!redirect_allowed(Some("https"), &https, 5, 5));
        assert!(!redirect_allowed(Some("http"), &http, 6, 5));
    }

    #[test]
    fn non_http_scheme_refused() {
        let ftp = reqwest::Url::parse("ftp://example.com/sub").unwrap();
        let file = reqwest::Url::parse("file:///etc/passwd").unwrap();
        assert!(!redirect_allowed(Some("https"), &ftp, 0, 5));
        assert!(!redirect_allowed(Some("http"), &file, 0, 5));
        assert!(!redirect_allowed(None, &ftp, 0, 5));
    }

    #[test]
    fn private_and_local_redirect_targets_refused() {
        for target in [
            "http://127.0.0.1:8080/sub",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.1.2.3/sub",
            "http://192.168.1.1/sub",
            "http://172.16.0.1/sub",
            "http://[::1]/sub",
            "http://[fe80::1]/sub",
            "http://[fc00::1]/sub",
            "http://0.0.0.0/sub",
            "http://localhost/sub",
            "http://sub.localhost/sub",
        ] {
            let url = reqwest::Url::parse(target).unwrap();
            assert!(
                !redirect_allowed(Some("https"), &url, 0, 5),
                "private redirect target allowed: {target}"
            );
        }
    }

    #[test]
    fn public_redirect_targets_allowed() {
        for target in [
            "https://example.com/sub",
            "https://sub.example.org/next",
            "http://198.51.100.7/sub",
            "https://[2001:db8::1]/sub",
            "https://example.com:8443/path?q=1",
        ] {
            let url = reqwest::Url::parse(target).unwrap();
            assert!(
                redirect_allowed(Some("http"), &url, 0, 5),
                "public redirect target refused: {target}"
            );
        }
    }
}
