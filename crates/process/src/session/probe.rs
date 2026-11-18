use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use v2ray_rs_core::models::{ProbeScheme, ProbeTarget, parse_probe_target};

/// Probes the local HTTP proxy without an HTTP client stack: a plain `GET` for
/// `http://` targets and a `CONNECT` tunnel for `https://` ones. The tunnel
/// reachability is what the health monitor needs; the TLS exchange belongs to
/// the browser or tool that eventually uses the proxy.
pub async fn probe_via_http_proxy(
    proxy: SocketAddr,
    url: &str,
    timeout: Duration,
) -> Result<(), String> {
    let target = parse_probe_target(url).map_err(|err| err.to_string())?;
    match tokio::time::timeout(timeout, exchange(proxy, &target)).await {
        Ok(result) => result,
        Err(_) => Err("timed out".to_string()),
    }
}

fn format_request(target: &ProbeTarget) -> String {
    match target.scheme {
        ProbeScheme::Http => format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            target.request_target, target.host_header
        ),
        ProbeScheme::Https => format!(
            "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n\r\n",
            target.host, target.port, target.host, target.port
        ),
    }
}

async fn exchange(proxy: SocketAddr, target: &ProbeTarget) -> Result<(), String> {
    let mut stream = TcpStream::connect(proxy)
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(format_request(target).as_bytes())
        .await
        .map_err(|error| error.to_string())?;

    let mut response = Vec::with_capacity(512);
    let mut chunk = [0u8; 512];
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        response.extend_from_slice(&chunk[..read]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") || response.len() > 8192 {
            break;
        }
    }

    status_code(&response).map_or_else(
        || Err("proxy closed the connection without a response".to_string()),
        |status| {
            let ok = match target.scheme {
                ProbeScheme::Https => (200..=299).contains(&status),
                ProbeScheme::Http => (200..=399).contains(&status),
            };
            if ok { Ok(()) } else { Err(status.to_string()) }
        },
    )
}

fn status_code(response: &[u8]) -> Option<u16> {
    let line_end = response.windows(2).position(|window| window == b"\r\n")?;
    let line = std::str::from_utf8(&response[..line_end]).ok()?;
    let mut parts = line.split_whitespace();
    let version = parts.next()?;
    if !version.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    const PROBE_URL: &str = "http://probe.test/generate_204";

    async fn spawn_proxy(response: Option<&'static str>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = socket.read(&mut buf).await;
                    match response {
                        Some(response) => {
                            let _ = socket.write_all(response.as_bytes()).await;
                        }
                        None => tokio::time::sleep(Duration::from_secs(5)).await,
                    }
                });
            }
        });
        addr
    }

    #[tokio::test]
    async fn probe_succeeds_on_204() {
        let proxy = spawn_proxy(Some("HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")).await;
        let result = probe_via_http_proxy(proxy, PROBE_URL, Duration::from_secs(5)).await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn probe_fails_on_502() {
        let proxy = spawn_proxy(Some(
            "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ))
        .await;
        let err = probe_via_http_proxy(proxy, PROBE_URL, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(err.contains("502"), "{err}");
    }

    #[tokio::test]
    async fn probe_fails_on_timeout() {
        let proxy = spawn_proxy(None).await;
        let err = probe_via_http_proxy(proxy, PROBE_URL, Duration::from_millis(300))
            .await
            .unwrap_err();
        assert!(err.contains("timed out"), "{err}");
    }

    #[tokio::test]
    async fn probe_fails_when_proxy_is_down() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = listener.local_addr().unwrap();
        drop(listener);
        let result = probe_via_http_proxy(proxy, PROBE_URL, Duration::from_secs(5)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn https_probe_uses_connect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let read = socket.read(&mut buf).await.unwrap();
            tx.send(String::from_utf8_lossy(&buf[..read]).to_string())
                .unwrap();
            let _ = socket
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await;
        });

        let result = probe_via_http_proxy(
            addr,
            "https://probe.test/generate_204",
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(result, Ok(()));
        assert!(rx.await.unwrap().starts_with("CONNECT probe.test:443"));
    }

    #[tokio::test]
    async fn http_probe_includes_non_default_port_in_host_header() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let read = socket.read(&mut buf).await.unwrap();
            tx.send(String::from_utf8_lossy(&buf[..read]).to_string())
                .unwrap();
            let _ = socket
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .await;
        });

        let result = probe_via_http_proxy(
            addr,
            "http://probe.example.com:8080/generate_204",
            Duration::from_secs(5),
        )
        .await;

        assert_eq!(result, Ok(()));
        let req = rx.await.unwrap();
        assert!(
            req.contains("Host: probe.example.com:8080\r\n"),
            "missing non-default port in Host header: {req:?}"
        );
    }

    #[tokio::test]
    async fn probe_rejects_1xx_and_non_2xx_connect() {
        let interim = spawn_proxy(Some("HTTP/1.1 100 Continue\r\n\r\n")).await;
        assert!(
            probe_via_http_proxy(interim, PROBE_URL, Duration::from_secs(5))
                .await
                .is_err()
        );

        let redirect = spawn_proxy(Some(
            "HTTP/1.1 302 Found\r\nLocation: https://example.com/\r\n\r\n",
        ))
        .await;
        assert!(
            probe_via_http_proxy(
                redirect,
                "https://probe.test/generate_204",
                Duration::from_secs(5)
            )
            .await
            .is_err()
        );
        assert!(
            probe_via_http_proxy(redirect, PROBE_URL, Duration::from_secs(5))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn probe_rejects_invalid_url_before_dialing() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        for bad in [
            "https://user:pass@example.com/",
            "https://[::1]extra/path",
            "https://example.com/path#frag",
        ] {
            let err = probe_via_http_proxy(addr, bad, Duration::from_secs(5))
                .await
                .unwrap_err();
            assert!(err.contains("invalid test URL"), "{bad}: {err}");
        }
    }
}
