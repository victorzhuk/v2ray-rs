use std::net::SocketAddr;
use std::time::Duration;

/// Issues a GET for `url` through the HTTP proxy listening at `proxy`.
///
/// Any status below 400 counts as success; transport errors, timeouts and
/// error statuses are returned as a human-readable reason.
pub async fn probe_via_http_proxy(
    proxy: SocketAddr,
    url: &str,
    timeout: Duration,
) -> Result<(), String> {
    let proxy = reqwest::Proxy::all(format!("http://{proxy}")).map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())?;

    match client.get(url).send().await {
        Ok(response) if response.status().as_u16() < 400 => Ok(()),
        Ok(response) => Err(response.status().to_string()),
        Err(e) if e.is_timeout() => Err("timed out".to_string()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const PROBE_URL: &str = "http://probe.test/generate_204";

    fn install_crypto_provider() {
        static RUSTLS_PROVIDER: Once = Once::new();
        RUSTLS_PROVIDER.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    async fn spawn_proxy(response: Option<&'static str>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = socket.read(&mut buf).await;
                    match response {
                        Some(r) => {
                            let _ = socket.write_all(r.as_bytes()).await;
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
        install_crypto_provider();
        let proxy = spawn_proxy(Some("HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")).await;
        let result = probe_via_http_proxy(proxy, PROBE_URL, Duration::from_secs(5)).await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn probe_fails_on_502() {
        install_crypto_provider();
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
        install_crypto_provider();
        let proxy = spawn_proxy(None).await;
        let err = probe_via_http_proxy(proxy, PROBE_URL, Duration::from_millis(300))
            .await
            .unwrap_err();
        assert!(err.contains("timed out"), "{err}");
    }

    #[tokio::test]
    async fn probe_fails_when_proxy_is_down() {
        install_crypto_provider();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = listener.local_addr().unwrap();
        drop(listener);
        let result = probe_via_http_proxy(proxy, PROBE_URL, Duration::from_secs(5)).await;
        assert!(result.is_err());
    }
}
