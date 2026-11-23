use std::error::Error as StdError;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use futures::StreamExt;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
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
    if hops >= cap || !target_is_public(next) {
        return false;
    }
    match next.scheme() {
        "https" => true,
        "http" => !matches!(scheme_from, Some("https")),
        _ => false,
    }
}

/// Shared public-target policy for redirect hops, initial subscription URLs,
/// and UI validation. DNS names are not resolved here; public names stay
/// allowed and private literals cannot masquerade as one.
fn target_is_public(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    // A trailing root dot names the same host (`localhost.` == `localhost`):
    // strip exactly one, or a refused name escapes as unclassified and an IP
    // literal skips the literal check.
    let host = host.strip_suffix('.').unwrap_or(host);
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = literal.parse::<std::net::IpAddr>() {
        return !ip_is_private_or_local(ip);
    }
    let host = host.to_ascii_lowercase();
    host != "localhost" && !host.ends_with(".localhost")
}

/// The one public subscription-URL predicate shared by the fetch path and the
/// UI forms: an `http`/`https` URL whose host is a public literal or a
/// non-localhost name. Any other input — unparsable text, foreign scheme,
/// private literal — is rejected, so classification logic cannot drift between
/// fetch-time enforcement and form-time feedback.
pub fn subscription_url_is_public(url: &str) -> bool {
    let parsed = match reqwest::Url::parse(url) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    matches!(parsed.scheme(), "http" | "https") && target_is_public(&parsed)
}

/// Carried through the resolver error chain so `fetch_with_client` can
/// classify a DNS-answer refusal as a non-transient `FetchError::InvalidUrl`
/// instead of a retryable network error.
#[derive(Debug)]
struct PrivateTargetRejected {
    host: String,
}

impl std::fmt::Display for PrivateTargetRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "host {} resolves to a private or local address",
            self.host
        )
    }
}

impl std::error::Error for PrivateTargetRejected {}

fn is_private_target_rejection(err: &reqwest::Error) -> bool {
    let mut source = err.source();
    while let Some(err) = source {
        if err.downcast_ref::<PrivateTargetRejected>().is_some() {
            return true;
        }
        source = err.source();
    }
    false
}

/// OS-backed lookup passed to [`PublicTargetsResolver`] in production.
struct TokioLookup;

impl Resolve for TokioLookup {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0u16))
                .await?
                .collect();
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// Connector-level resolver refusing every answer that contains a private or
/// local address. Wired into the production subscription client so a
/// rebinding DNS answer cannot redirect a connect, including on redirect
/// hops where only the URL string was validated. A single private answer
/// rejects the whole lookup: partial filtering would let an attacker mix
/// public answers with the private one they want reached.
pub(crate) struct PublicTargetsResolver {
    inner: Arc<dyn Resolve>,
}

impl Default for PublicTargetsResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl PublicTargetsResolver {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(TokioLookup),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_inner(inner: Arc<dyn Resolve>) -> Self {
        Self { inner }
    }
}

impl Resolve for PublicTargetsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let inner = self.inner.clone();
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let addrs: Vec<SocketAddr> = inner.resolve(name).await?.collect();
            if addrs.iter().any(|addr| ip_is_private_or_local(addr.ip())) {
                return Err(Box::new(PrivateTargetRejected { host })
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

fn ip_is_private_or_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            let is_cgnat = octets[0] == 100 && (octets[1] & 0xc0) == 64;
            let is_benchmarking = octets[0] == 198 && (octets[1] & 0xfe) == 18;
            let is_this_network = octets[0] == 0;
            let is_multicast = octets[0] & 0xf0 == 224;
            let is_reserved = octets[0] >= 240;
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || is_cgnat
                || is_benchmarking
                || is_this_network
                || is_multicast
                || is_reserved
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return ip_is_private_or_local(std::net::IpAddr::V4(v4));
            }
            let segments = v6.segments();
            let is_nat64 = segments[..6] == [0x0064, 0xff9b, 0, 0, 0, 0];
            if is_nat64 {
                let v4 = std::net::Ipv4Addr::new(
                    (segments[6] >> 8) as u8,
                    segments[6] as u8,
                    (segments[7] >> 8) as u8,
                    segments[7] as u8,
                );
                return ip_is_private_or_local(std::net::IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unicast_link_local()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xff00) == 0xff00
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
            } else if !target_is_public(attempt.url()) {
                format!("refusing redirect to a private or local target (host {host})")
            } else {
                format!("refusing insecure or unsupported redirect for host {host}")
            };
            attempt.error(reason)
        }
    })
}

pub const MAX_SUBSCRIPTION_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
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
    fetch_with_targets_policy(client, url, true).await
}

/// Loopback-only fixtures; skips the initial public-target check so tests can
/// serve subscriptions from `127.0.0.1`. Production callers never see this
/// path: the equivalent service constructor is itself
/// `#[cfg(any(test, feature = "test-utils"))]`-gated.
#[cfg(any(test, feature = "test-utils"))]
pub async fn fetch_with_client_allow_private(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, FetchError> {
    fetch_with_targets_policy(client, url, false).await
}

async fn fetch_with_targets_policy(
    client: &reqwest::Client,
    url: &str,
    enforce_public_targets: bool,
) -> Result<String, FetchError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| FetchError::InvalidUrl(e.to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(FetchError::InvalidUrl(format!(
            "unsupported scheme: {} (only http/https)",
            parsed.scheme()
        )));
    }
    if enforce_public_targets && !target_is_public(&parsed) {
        return Err(FetchError::InvalidUrl(format!(
            "refusing subscription target that is not a public host (host {})",
            parsed.host_str().unwrap_or("unknown host")
        )));
    }
    if parsed.scheme() == "http" {
        log::warn!("fetching subscription over plaintext HTTP — credentials may be exposed");
    }

    let response = client.get(parsed).send().await.map_err(|e| {
        if is_private_target_rejection(&e) {
            let host = e
                .url()
                .and_then(|u| u.host_str())
                .unwrap_or("unknown host")
                .to_owned();
            FetchError::InvalidUrl(format!(
                "refusing DNS answer resolving to a private or local address (host {host})"
            ))
        } else if e.is_timeout() {
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
    use std::net::IpAddr;
    use std::sync::Once;

    #[test]
    fn test_ip_guard_rejects_special_ranges() {
        for ip in [
            "0.0.0.1",
            "0.255.255.255",
            "224.0.0.1",
            "240.0.0.1",
            "ff02::1",
            "64:ff9b::7f00:1",
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(
                ip_is_private_or_local(ip),
                "{ip} must be rejected as private or local"
            );
        }
    }

    #[test]
    fn test_ip_guard_allows_public_addresses() {
        for ip in ["1.1.1.1", "2606:4700:4700::1111", "64:ff9b::808:808"] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(
                !ip_is_private_or_local(ip),
                "{ip} must not be rejected as private or local"
            );
        }
    }

    #[test]
    fn test_ip_guard_nat64_embeds_ipv4_rules() {
        let nat64_loopback: IpAddr = "64:ff9b::7f00:1".parse().unwrap();
        assert!(ip_is_private_or_local(nat64_loopback));
        let nat64_public: IpAddr = "64:ff9b::808:808".parse().unwrap();
        assert!(!ip_is_private_or_local(nat64_public));
    }

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
        let result = fetch_with_client_allow_private(&client, &url).await;
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
        let result = fetch_with_client_allow_private(&client, &format!("http://{addr}/sub")).await;
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
            "https://100.64.0.1/sub",
            "https://100.127.255.254/sub",
            "https://198.18.0.1/sub",
            "https://198.19.255.254/sub",
            "https://[::ffff:100.64.0.1]/sub",
            "https://[::ffff:198.18.0.1]/sub",
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

    #[test]
    fn subscription_url_predicate_matches_target_policy() {
        for url in [
            "http://127.0.0.1:8080/sub",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/sub",
            "http://[::ffff:10.0.0.1]/sub",
            "https://localhost/sub",
            "https://sub.localhost/sub",
            "http://localhost./sub",
            "http://x.localhost./sub",
            "http://127.0.0.1./sub",
            "http://10.1.2.3/sub",
            "http://192.168.1.1/sub",
            "http://172.16.0.1/sub",
            "https://100.64.0.1/sub",
            "https://198.18.0.1/sub",
            "http://0.0.0.0/sub",
            "http://255.255.255.255/sub",
            "http://[fe80::1]/sub",
            "http://[fc00::1]/sub",
            "ftp://example.com/sub",
            "not a url",
        ] {
            assert!(!subscription_url_is_public(url), "accepted: {url}");
        }
        for url in [
            "https://example.com/sub",
            "http://198.51.100.7/sub",
            "https://[2001:db8::1]/sub",
        ] {
            assert!(subscription_url_is_public(url), "refused: {url}");
        }
    }

    #[tokio::test]
    async fn fetch_with_client_rejects_private_initial_urls() {
        let client = test_client();
        for url in [
            "http://127.0.0.1:8080/sub",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]:8080/sub",
            "http://[::ffff:10.0.0.1]/sub",
            "https://localhost/sub",
        ] {
            let err = fetch_with_client(&client, url).await.unwrap_err();
            assert!(matches!(err, FetchError::InvalidUrl(_)), "{url}: {err:?}");
            assert!(!err.is_transient(), "{url}: refusal must not retry");
        }
    }

    struct StaticResolver {
        addrs: Vec<std::net::SocketAddr>,
    }

    impl Resolve for StaticResolver {
        fn resolve(&self, _name: Name) -> Resolving {
            let addrs = self.addrs.clone();
            Box::pin(std::future::ready(Ok(Box::new(addrs.into_iter()) as Addrs)))
        }
    }

    #[tokio::test]
    async fn resolver_rejects_any_private_dns_answer() {
        let resolver = PublicTargetsResolver::with_inner(Arc::new(StaticResolver {
            addrs: vec!["8.8.8.8:0".parse().unwrap(), "127.0.0.1:0".parse().unwrap()],
        }));
        let name = std::str::FromStr::from_str("rebind.example.com").unwrap();

        let result = resolver.resolve(name).await;

        assert!(
            result.is_err(),
            "a lookup containing a private answer must be rejected"
        );
    }

    #[tokio::test]
    async fn resolver_allows_public_dns_answers() {
        let resolver = PublicTargetsResolver::with_inner(Arc::new(StaticResolver {
            addrs: vec!["8.8.8.8:0".parse().unwrap(), "1.1.1.1:0".parse().unwrap()],
        }));
        let name = std::str::FromStr::from_str("example.com").unwrap();

        let addrs = resolver.resolve(name).await.unwrap();

        assert_eq!(addrs.count(), 2);
    }
}
