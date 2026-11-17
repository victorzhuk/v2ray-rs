use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Probes the local HTTP proxy without an HTTP client stack: a plain `GET` for
/// `http://` targets and a `CONNECT` tunnel for `https://` ones. The tunnel
/// reachability is what the health monitor needs; the TLS exchange belongs to
/// the browser or tool that eventually uses the proxy.
pub async fn probe_via_http_proxy(
    proxy: SocketAddr,
    url: &str,
    timeout: Duration,
) -> Result<(), String> {
    let target = Target::parse(url)?;
    match tokio::time::timeout(timeout, exchange(proxy, &target)).await {
        Ok(result) => result,
        Err(_) => Err("timed out".to_string()),
    }
}

struct Target {
    scheme: Scheme,
    authority: String,
    host: String,
    port: u16,
    path: String,
}

#[derive(PartialEq)]
enum Scheme {
    Http,
    Https,
}

impl Target {
    fn parse(url: &str) -> Result<Self, String> {
        let (scheme, rest) = if let Some(rest) = url.strip_prefix("http://") {
            (Scheme::Http, rest)
        } else if let Some(rest) = url.strip_prefix("https://") {
            (Scheme::Https, rest)
        } else {
            return Err(format!("unsupported probe URL: {url}"));
        };
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if authority.is_empty() {
            return Err(format!("probe URL has no host: {url}"));
        }

        let (host, port) = split_authority(authority);
        let port = port.unwrap_or(if scheme == Scheme::Https { 443 } else { 80 });
        Ok(Self {
            scheme,
            authority: authority.to_string(),
            host,
            port,
            path: format!("/{path}"),
        })
    }

    fn request(&self) -> String {
        match self.scheme {
            Scheme::Http => format!(
                "GET http://{}{} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                self.authority, self.path, self.host
            ),
            Scheme::Https => format!(
                "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n\r\n",
                self.host, self.port, self.host, self.port
            ),
        }
    }
}

fn split_authority(authority: &str) -> (String, Option<u16>) {
    if let Some(rest) = authority.strip_prefix('[')
        && let Some((host, tail)) = rest.split_once(']')
    {
        let port = tail.strip_prefix(':').and_then(|port| port.parse().ok());
        return (format!("[{host}]"), port);
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => match port.parse() {
            Ok(port) => (host.to_string(), Some(port)),
            Err(_) => (authority.to_string(), None),
        },
        _ => (authority.to_string(), None),
    }
}

async fn exchange(proxy: SocketAddr, target: &Target) -> Result<(), String> {
    let mut stream = TcpStream::connect(proxy)
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(target.request().as_bytes())
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
            if status < 400 {
                Ok(())
            } else {
                Err(status.to_string())
            }
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
}
