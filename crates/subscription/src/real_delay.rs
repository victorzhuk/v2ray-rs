//! On-demand "Real Delay" probe orchestration.
//!
//! A Real Delay probe spawns a short-lived, isolated backend instance with a
//! generated config (see [`v2ray_rs_core::config::probe`]), drives the
//! backend's own delay-test surface to measure the end-to-end latency of an
//! HTTP request **through** each candidate node, then shuts the instance down.
//!
//! v2ray-rs never implements proxy protocols itself: the dial is performed by
//! the installed backend binary.

use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use tokio::sync::Mutex;

use crate::observatory::{
    ObservatoryError, ObservatoryStatus, query_v2ray_observatory, query_xray_observatory,
};
use std::time::Instant;
use v2ray_rs_core::config::{probe_generator_for, probe_tag};
use v2ray_rs_core::models::{BackendType, RealDelaySettings, SubscriptionNode};
use v2ray_rs_core::persistence::AppPaths;
use v2ray_rs_process::{ProbeError, ProbeRunner};

/// At most one ephemeral probe backend may exist at a time per process, to
/// bound resource use and avoid port conflicts between concurrent sessions.
static REAL_DELAY_LOCK: Mutex<()> = Mutex::const_new(());

const WAIT_READY_TIMEOUT: Duration = Duration::from_secs(2);

/// Outcome of a Real Delay session: one result per input node (in order) plus
/// an optional human-readable diagnostic for the UI to surface as a toast.
#[derive(Debug, Clone)]
pub struct RealDelayReport {
    pub results: Vec<Option<u64>>,
    pub diagnostic: Option<String>,
}

impl RealDelayReport {
    fn failed(len: usize, diagnostic: impl Into<String>) -> Self {
        Self {
            results: vec![None; len],
            diagnostic: Some(diagnostic.into()),
        }
    }
}

/// Result from polling the observatory service.
///
/// Contains the collected delays and an optional diagnostic if the service
/// failed to respond entirely.
#[derive(Debug, Clone)]
struct ObservatoryDelaysResult {
    results: Vec<Option<u64>>,
    diagnostic: Option<String>,
}

/// Measures the Real Delay of each node through an ephemeral backend instance.
///
/// Returns one `Option<u64>` (milliseconds) per input node, in order; `None`
/// indicates a failed or timed-out probe for that node. Total failures (the
/// backend can't start, the API never becomes reachable, or the backend lacks
/// the required delay-test surface) yield all-`None` plus a `diagnostic`.
pub async fn measure_real_delay(
    backend: BackendType,
    binary: &Path,
    nodes: &[&SubscriptionNode],
    cfg: &RealDelaySettings,
    paths: &AppPaths,
) -> RealDelayReport {
    let len = nodes.len();
    if len == 0 {
        return RealDelayReport {
            results: Vec::new(),
            diagnostic: None,
        };
    }

    let Some(generator) = probe_generator_for(backend) else {
        return RealDelayReport::failed(len, format!("Real Delay is not supported by {backend}"));
    };

    // Serialize the whole session so only one ephemeral backend runs at a time.
    let _guard = REAL_DELAY_LOCK.lock().await;

    if let Err(e) = paths.ensure_dirs() {
        return RealDelayReport::failed(len, format!("failed to prepare runtime dir: {e}"));
    }

    let Some(api_port) = pick_free_loopback_port() else {
        return RealDelayReport::failed(len, "could not allocate a loopback probe port");
    };

    let config = generator.generate(nodes, api_port, &cfg.test_url, cfg.timeout_ms);
    let config_json = match serde_json::to_vec_pretty(&config) {
        Ok(bytes) => bytes,
        Err(e) => {
            return RealDelayReport::failed(len, format!("failed to serialize probe config: {e}"));
        }
    };

    let temp = match tempfile::Builder::new()
        .prefix("real-delay-")
        .suffix(".json")
        .tempfile_in(paths.runtime_dir())
    {
        Ok(mut f) => {
            use std::io::Write;
            if let Err(e) = f.write_all(&config_json) {
                return RealDelayReport::failed(len, format!("failed to write probe config: {e}"));
            }
            f
        }
        Err(e) => {
            return RealDelayReport::failed(len, format!("failed to create probe config: {e}"));
        }
    };

    let mut runner = ProbeRunner::new(binary.to_path_buf(), temp.path().to_path_buf(), api_port);

    if let Err(e) = runner.start().await {
        return RealDelayReport::failed(len, format!("failed to start probe backend: {e}"));
    }

    if let Err(e) = runner.wait_ready(WAIT_READY_TIMEOUT).await {
        let _ = runner.stop().await;
        let diagnostic = match e {
            ProbeError::ApiNotReady(_) => {
                format!("Backend does not support real-delay tests ({backend} API required)")
            }
            other => format!("probe backend error: {other}"),
        };
        return RealDelayReport::failed(len, diagnostic);
    }

    match backend {
        BackendType::SingBox => {
            let results = singbox_delays(api_port, len, &cfg.test_url, cfg.timeout_ms).await;
            let _ = runner.stop().await;
            drop(temp);
            RealDelayReport {
                results,
                diagnostic: None,
            }
        }
        BackendType::Xray => {
            let obs_result = observatory_delays_with_query(len, cfg.timeout_ms, || {
                query_xray_observatory(api_port)
            })
            .await;
            let _ = runner.stop().await;
            drop(temp);
            RealDelayReport {
                results: obs_result.results,
                diagnostic: obs_result.diagnostic,
            }
        }
        BackendType::V2ray => {
            let obs_result = observatory_delays_with_query(len, cfg.timeout_ms, || {
                query_v2ray_observatory(api_port)
            })
            .await;
            let _ = runner.stop().await;
            drop(temp);
            RealDelayReport {
                results: obs_result.results,
                diagnostic: obs_result.diagnostic,
            }
        }
    }
}

/// Binds `127.0.0.1:0`, reads the assigned port, then closes the socket so the
/// ephemeral backend can claim it.
fn pick_free_loopback_port() -> Option<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
}

/// Issues parallel `GET /proxies/{tag}/delay` requests against the sing-box
/// Clash API and collects per-node results.
async fn singbox_delays(
    api_port: u16,
    count: usize,
    test_url: &str,
    timeout_ms: u32,
) -> Vec<Option<u64>> {
    let client = match reqwest::Client::builder().no_proxy().build() {
        Ok(c) => c,
        Err(_) => return vec![None; count],
    };
    let encoded_url = utf8_percent_encode(test_url, NON_ALPHANUMERIC).to_string();
    // Allow the backend's own per-probe timeout to elapse before we give up.
    let request_timeout = Duration::from_millis(u64::from(timeout_ms) + 3000);

    let mut handles = Vec::with_capacity(count);
    for i in 0..count {
        let client = client.clone();
        let url = format!(
            "http://127.0.0.1:{api_port}/proxies/{tag}/delay?url={encoded_url}&timeout={timeout_ms}",
            tag = probe_tag(i),
        );
        handles.push(tokio::spawn(async move {
            query_singbox_delay(&client, &url, request_timeout).await
        }));
    }

    let mut results = Vec::with_capacity(count);
    for handle in handles {
        results.push(handle.await.ok().flatten());
    }
    results
}

async fn query_singbox_delay(
    client: &reqwest::Client,
    url: &str,
    timeout: Duration,
) -> Option<u64> {
    let response = client.get(url).timeout(timeout).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;
    let body: serde_json::Value = serde_json::from_str(&text).ok()?;
    body.get("delay").and_then(serde_json::Value::as_u64)
}

/// Parses "probe-<idx>" tag suffix to extract the node index.
fn parse_probe_index(tag: &str) -> Option<usize> {
    tag.strip_prefix(v2ray_rs_core::config::PROBE_TAG_PREFIX)
        .and_then(|suffix| suffix.parse::<usize>().ok())
}

/// Internal version of observatory_delays that accepts a query function for testing.
///
/// This function polls the observatory repeatedly until either:
/// - All expected probe tags have results
/// - The deadline expires
///
/// Returns an `ObservatoryDelaysResult` with results and an optional diagnostic.
/// A diagnostic is produced only if:
/// - Zero results were collected
/// - At least one gRPC error occurred during polling
async fn observatory_delays_with_query<F, Fut>(
    count: usize,
    timeout_ms: u32,
    query_fn: F,
) -> ObservatoryDelaysResult
where
    F: Fn() -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<Vec<ObservatoryStatus>, ObservatoryError>> + Send,
{
    // Minimum 2-second polling window
    let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms + 1500).max(2000));
    let poll_interval = Duration::from_millis(300);

    let mut results: Vec<Option<u64>> = vec![None; count];
    let mut had_grpc_error = false;

    loop {
        let query_result = query_fn().await;

        match query_result {
            Ok(statuses) => {
                for status in &statuses {
                    if let Some(idx) = parse_probe_index(&status.outbound_tag)
                        && idx < count
                    {
                        results[idx] = status.delay_ms;
                    }
                }
                // Check if all results collected
                if results.iter().all(|r| r.is_some()) {
                    break;
                }
            }
            Err(_) => {
                // gRPC not ready yet, continue polling
                had_grpc_error = true;
            }
        }

        if Instant::now() >= deadline {
            break;
        }

        tokio::time::sleep(poll_interval).await;
    }

    // Produce diagnostic if we got zero results and had gRPC errors
    let diagnostic = if results.iter().all(|r| r.is_none()) && had_grpc_error {
        Some(
            "Observatory service did not respond — the backend may not support ObservatoryService"
                .to_string(),
        )
    } else {
        None
    };

    ObservatoryDelaysResult {
        results,
        diagnostic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener as TokioListener;

    fn install_crypto_provider() {
        static RUSTLS_PROVIDER: Once = Once::new();
        RUSTLS_PROVIDER.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    /// Minimal mock Clash API: for `GET /proxies/probe-<i>/delay`, returns the
    /// i-th canned response. A `None` response yields HTTP 408.
    async fn spawn_mock_clash(responses: Vec<Option<u64>>) -> u16 {
        let listener = TokioListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let responses = responses.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let first_line = req.lines().next().unwrap_or("");
                    let idx = first_line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|path| path.strip_prefix("/proxies/probe-"))
                        .and_then(|rest| rest.split('/').next())
                        .and_then(|num| num.parse::<usize>().ok());

                    let response = match idx.and_then(|i| responses.get(i).copied()) {
                        Some(Some(delay)) => {
                            let body = format!("{{\"delay\":{delay}}}");
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                        _ => {
                            let body = "{\"message\":\"timeout\"}";
                            format!(
                                "HTTP/1.1 408 Request Timeout\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                    };
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.flush().await;
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn singbox_delays_success_and_partial_failures() {
        install_crypto_provider();
        let port = spawn_mock_clash(vec![Some(120), None, Some(248)]).await;
        let results = singbox_delays(port, 3, "https://www.gstatic.com/generate_204", 5000).await;
        assert_eq!(results, vec![Some(120), None, Some(248)]);
    }

    #[tokio::test]
    async fn singbox_delays_all_failures_when_server_absent() {
        install_crypto_provider();
        // Bind and immediately drop to get a dead port.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let results = singbox_delays(port, 2, "https://www.gstatic.com/generate_204", 200).await;
        assert_eq!(results, vec![None, None]);
    }

    #[tokio::test]
    async fn measure_real_delay_empty_nodes() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_profile_in(v2ray_rs_core::profile::AppProfile::Test, dir.path());
        let report = measure_real_delay(
            BackendType::SingBox,
            Path::new("/nonexistent/sing-box"),
            &[],
            &RealDelaySettings::default(),
            &paths,
        )
        .await;
        assert!(report.results.is_empty());
        assert!(report.diagnostic.is_none());
    }

    #[tokio::test]
    async fn measure_real_delay_unsupported_backend() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_profile_in(v2ray_rs_core::profile::AppProfile::Test, dir.path());
        let node = SubscriptionNode::new(v2ray_rs_core::models::ProxyNode::Shadowsocks(
            v2ray_rs_core::models::ShadowsocksConfig {
                address: "ss.example.com".into(),
                port: 8388,
                method: "aes-256-gcm".into(),
                password: "pw".into(),
                remark: None,
            },
        ));
        let report = measure_real_delay(
            BackendType::V2ray,
            Path::new("/nonexistent/v2ray"),
            &[&node],
            &RealDelaySettings::default(),
            &paths,
        )
        .await;
        assert_eq!(report.results, vec![None]);
        assert!(report.diagnostic.is_some());
    }

    #[test]
    fn test_parse_probe_index() {
        // Valid probe tags
        assert_eq!(parse_probe_index("probe-0"), Some(0));
        assert_eq!(parse_probe_index("probe-5"), Some(5));
        assert_eq!(parse_probe_index("probe-123"), Some(123));

        // Invalid probe tags
        assert_eq!(parse_probe_index("other"), None);
        assert_eq!(parse_probe_index("probe-abc"), None);
        assert_eq!(parse_probe_index("probe--1"), None);
        assert_eq!(parse_probe_index("probe-"), None);
        assert_eq!(parse_probe_index("xprobe-0"), None);
        assert_eq!(parse_probe_index(""), None);
    }

    #[test]
    fn test_map_observatory_results() {
        use crate::observatory::ObservatoryStatus;

        // Helper function similar to the one in observatory_delays
        fn map_observatory_results(
            statuses: &[ObservatoryStatus],
            count: usize,
        ) -> Vec<Option<u64>> {
            let mut results = vec![None; count];
            for status in statuses {
                if let Some(idx) = parse_probe_index(&status.outbound_tag) {
                    if idx < count {
                        results[idx] = status.delay_ms;
                    }
                }
            }
            results
        }

        // All successful
        let statuses = vec![
            ObservatoryStatus {
                outbound_tag: "probe-0".to_string(),
                delay_ms: Some(150),
                alive: true,
                last_error: None,
            },
            ObservatoryStatus {
                outbound_tag: "probe-1".to_string(),
                delay_ms: Some(300),
                alive: true,
                last_error: None,
            },
            ObservatoryStatus {
                outbound_tag: "probe-2".to_string(),
                delay_ms: Some(200),
                alive: true,
                last_error: None,
            },
        ];
        let results = map_observatory_results(&statuses, 3);
        assert_eq!(results, vec![Some(150), Some(300), Some(200)]);

        // Partial success (some nodes missing)
        let statuses = vec![
            ObservatoryStatus {
                outbound_tag: "probe-0".to_string(),
                delay_ms: Some(150),
                alive: true,
                last_error: None,
            },
            // probe-1 is missing
            ObservatoryStatus {
                outbound_tag: "probe-2".to_string(),
                delay_ms: Some(200),
                alive: true,
                last_error: None,
            },
        ];
        let results = map_observatory_results(&statuses, 3);
        assert_eq!(results, vec![Some(150), None, Some(200)]);

        // Unknown tags ignored
        let statuses = vec![
            ObservatoryStatus {
                outbound_tag: "probe-0".to_string(),
                delay_ms: Some(150),
                alive: true,
                last_error: None,
            },
            ObservatoryStatus {
                outbound_tag: "other-tag".to_string(),
                delay_ms: Some(300),
                alive: true,
                last_error: None,
            },
            ObservatoryStatus {
                outbound_tag: "probe-2".to_string(),
                delay_ms: Some(200),
                alive: true,
                last_error: None,
            },
        ];
        let results = map_observatory_results(&statuses, 3);
        assert_eq!(results, vec![Some(150), None, Some(200)]);

        // Mixed alive/dead nodes
        let statuses = vec![
            ObservatoryStatus {
                outbound_tag: "probe-0".to_string(),
                delay_ms: Some(150),
                alive: true,
                last_error: None,
            },
            ObservatoryStatus {
                outbound_tag: "probe-1".to_string(),
                delay_ms: None, // Dead node
                alive: false,
                last_error: Some("connection refused".to_string()),
            },
            ObservatoryStatus {
                outbound_tag: "probe-2".to_string(),
                delay_ms: Some(200),
                alive: true,
                last_error: None,
            },
        ];
        let results = map_observatory_results(&statuses, 3);
        assert_eq!(results, vec![Some(150), None, Some(200)]);

        // Index out of bounds ignored
        let statuses = vec![
            ObservatoryStatus {
                outbound_tag: "probe-0".to_string(),
                delay_ms: Some(150),
                alive: true,
                last_error: None,
            },
            ObservatoryStatus {
                outbound_tag: "probe-5".to_string(), // Out of bounds for count=3
                delay_ms: Some(300),
                alive: true,
                last_error: None,
            },
            ObservatoryStatus {
                outbound_tag: "probe-2".to_string(),
                delay_ms: Some(200),
                alive: true,
                last_error: None,
            },
        ];
        let results = map_observatory_results(&statuses, 3);
        assert_eq!(results, vec![Some(150), None, Some(200)]);
    }

    #[tokio::test]
    async fn observatory_delays_full_success() {
        // Mock that returns all results on first query
        use std::sync::atomic::{AtomicUsize, Ordering};
        let call_count = std::sync::Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();
        let query_fn = move || {
            let cc = cc.clone();
            async move {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(vec![
                    ObservatoryStatus {
                        outbound_tag: "probe-0".to_string(),
                        delay_ms: Some(150),
                        alive: true,
                        last_error: None,
                    },
                    ObservatoryStatus {
                        outbound_tag: "probe-1".to_string(),
                        delay_ms: Some(300),
                        alive: true,
                        last_error: None,
                    },
                    ObservatoryStatus {
                        outbound_tag: "probe-2".to_string(),
                        delay_ms: Some(200),
                        alive: true,
                        last_error: None,
                    },
                ])
            }
        };

        let result = observatory_delays_with_query(3, 5000, query_fn).await;
        assert_eq!(result.results, vec![Some(150), Some(300), Some(200)]);
        assert!(result.diagnostic.is_none());
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // Should stop after first successful query
    }

    #[tokio::test]
    async fn observatory_delays_partial_success() {
        // Mock that returns partial results on first query, then completes
        use std::sync::atomic::{AtomicUsize, Ordering};
        let call_count = std::sync::Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();
        let query_fn = move || {
            let cc = cc.clone();
            async move {
                let c = cc.fetch_add(1, Ordering::SeqCst) + 1;
                if c == 1 {
                    // First query: only probe-0 and probe-2 have results
                    Ok(vec![
                        ObservatoryStatus {
                            outbound_tag: "probe-0".to_string(),
                            delay_ms: Some(150),
                            alive: true,
                            last_error: None,
                        },
                        ObservatoryStatus {
                            outbound_tag: "probe-2".to_string(),
                            delay_ms: Some(200),
                            alive: true,
                            last_error: None,
                        },
                    ])
                } else {
                    // Second query: probe-1 arrives
                    Ok(vec![
                        ObservatoryStatus {
                            outbound_tag: "probe-0".to_string(),
                            delay_ms: Some(150),
                            alive: true,
                            last_error: None,
                        },
                        ObservatoryStatus {
                            outbound_tag: "probe-1".to_string(),
                            delay_ms: Some(300),
                            alive: true,
                            last_error: None,
                        },
                        ObservatoryStatus {
                            outbound_tag: "probe-2".to_string(),
                            delay_ms: Some(200),
                            alive: true,
                            last_error: None,
                        },
                    ])
                }
            }
        };

        let result = observatory_delays_with_query(3, 5000, query_fn).await;
        assert_eq!(result.results, vec![Some(150), Some(300), Some(200)]);
        assert!(result.diagnostic.is_none());
        assert!(call_count.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn observatory_delays_all_timeout_no_results() {
        // Mock that always fails with a connection error
        let query_fn = || async move {
            Err(ObservatoryError::Status(tonic::Status::unavailable(
                "connection refused",
            )))
        };

        let result = observatory_delays_with_query(3, 5000, query_fn).await;
        assert_eq!(result.results, vec![None, None, None]);
        assert!(result.diagnostic.is_some());
        assert!(
            result
                .diagnostic
                .unwrap()
                .contains("Observatory service did not respond")
        );
    }

    #[tokio::test]
    async fn observatory_delays_grpc_error_with_partial_results() {
        // Mock that fails first few times, then returns partial results
        use std::sync::atomic::{AtomicUsize, Ordering};
        let call_count = std::sync::Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();
        let query_fn = move || {
            let cc = cc.clone();
            async move {
                let c = cc.fetch_add(1, Ordering::SeqCst) + 1;
                if c < 3 {
                    Err(ObservatoryError::Status(tonic::Status::unavailable(
                        "connection refused",
                    )))
                } else {
                    // Eventually return some results
                    Ok(vec![
                        ObservatoryStatus {
                            outbound_tag: "probe-0".to_string(),
                            delay_ms: Some(150),
                            alive: true,
                            last_error: None,
                        },
                        // probe-1 and probe-2 never arrive
                    ])
                }
            }
        };

        let result = observatory_delays_with_query(3, 5000, query_fn).await;
        assert_eq!(result.results, vec![Some(150), None, None]);
        // No diagnostic because we got at least one result
        assert!(result.diagnostic.is_none());
    }

    #[tokio::test]
    async fn observatory_delays_immediate_completion() {
        // Mock that returns all results immediately (even faster than poll interval)
        use std::sync::atomic::{AtomicUsize, Ordering};
        let call_count = std::sync::Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();
        let query_fn = move || {
            let cc = cc.clone();
            async move {
                cc.fetch_add(1, Ordering::SeqCst);
                Ok(vec![
                    ObservatoryStatus {
                        outbound_tag: "probe-0".to_string(),
                        delay_ms: Some(100),
                        alive: true,
                        last_error: None,
                    },
                    ObservatoryStatus {
                        outbound_tag: "probe-1".to_string(),
                        delay_ms: Some(200),
                        alive: true,
                        last_error: None,
                    },
                ])
            }
        };

        let result = observatory_delays_with_query(2, 100, query_fn).await;
        assert_eq!(result.results, vec![Some(100), Some(200)]);
        assert!(result.diagnostic.is_none());
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn observatory_delays_minimum_deadline_guard() {
        // Test that even with a tiny timeout, we get at least 2 seconds of polling
        use std::time::Instant;

        let start = Instant::now();
        let query_fn = || async move {
            Err(ObservatoryError::Status(tonic::Status::unavailable(
                "connection refused",
            )))
        };

        // Use a very small timeout (10ms) - should still poll for at least 2 seconds
        let result = observatory_delays_with_query(2, 10, query_fn).await;
        let elapsed = start.elapsed();

        assert_eq!(result.results, vec![None, None]);
        assert!(result.diagnostic.is_some());
        assert!(
            elapsed.as_millis() >= 1900,
            "Minimum deadline guard failed: elapsed only {}ms",
            elapsed.as_millis()
        );
    }
}
