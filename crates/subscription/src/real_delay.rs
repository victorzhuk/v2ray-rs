//! On-demand "Real Delay" probe orchestration.
//!
//! A Real Delay probe spawns a short-lived, isolated backend instance with a
//! generated config (see [`v2ray_rs_core::config::probe`]), drives the
//! backend's own delay-test surface to measure the end-to-end latency of an
//! HTTP request **through** each candidate node, then shuts the instance down.
//!
//! v2ray-rs never implements proxy protocols itself: the dial is performed by
//! the installed backend binary.

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use tokio::sync::Mutex;

use crate::fetch::redirect_policy;
use crate::observatory::{
    ObservatoryError, ObservatoryStatus, query_v2ray_observatory, query_xray_observatory,
};
use chrono::Utc;
use serde_json::Value;
use std::time::Instant;
use v2ray_rs_core::config::{
    egress_inbound_ports, pick_free_loopback_ports, probe_generator_for, probe_tag,
};
use v2ray_rs_core::geo_probe::{AvailabilityVerdict, ServiceCheck, derive_verdict};
use v2ray_rs_core::models::{
    BackendType, EgressSample, RealDelaySettings, SubscriptionNode, VerdictRecord,
};
use v2ray_rs_core::persistence::AppPaths;
use v2ray_rs_process::{ProbeError, ProbeRunner};

/// At most one ephemeral probe backend may exist at a time per process, to
/// bound resource use and avoid port conflicts between concurrent sessions.
/// Both Real Delay and availability sessions take this lock, so an availability
/// request during a running session queues instead of spawning a second
/// ephemeral backend.
static REAL_DELAY_LOCK: Mutex<()> = Mutex::const_new(());

const WAIT_READY_TIMEOUT: Duration = Duration::from_secs(2);

/// IP-echo endpoint queried through each node's tunnel to observe its egress.
/// Anycast, key-less, not geo-fenced, plain-text answer (`ip=…`, `loc=…`).
const EGRESS_ECHO_URL: &str = "https://www.cloudflare.com/cdn-cgi/trace";

/// Conventional browser identity: Google serves different gate pages to
/// unidentified clients.
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";

/// Redirect hops followed for an availability check.
const REDIRECT_CAP: usize = 5;
/// Body cap for an availability check reply.
const AVAILABILITY_BODY_CAP: usize = 2 * 1024 * 1024;
/// Body cap for the IP-echo reply.
const EGRESS_BODY_CAP: usize = 64 * 1024;

/// Upper bound on per-node probe futures in flight at once. Shared by the
/// delay pass and the sing-box API pass so a large node list does not put
/// every node's probe in flight simultaneously.
const MAX_CONCURRENT_NODE_PROBES: usize = 8;

/// Outcome of a Real Delay session: one result per input node (in order) plus
/// an optional human-readable diagnostic for the UI to surface as a toast.
/// The egress and verdict vectors run parallel to `results`; a node without an
/// egress inbound has `None` / an empty map there.
#[derive(Debug, Clone)]
pub struct RealDelayReport {
    pub results: Vec<Option<u64>>,
    pub egress: Vec<Option<EgressSample>>,
    pub verdicts: Vec<HashMap<String, VerdictRecord>>,
    pub diagnostic: Option<String>,
}

impl RealDelayReport {
    fn failed(len: usize, diagnostic: impl Into<String>) -> Self {
        Self {
            results: vec![None; len],
            egress: vec![None; len],
            verdicts: vec![HashMap::new(); len],
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

/// Measures the Real Delay of each node through an ephemeral backend instance,
/// also observing each node's egress. No availability checks are run; use
/// [`check_node_availability`] for that.
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
    run_probe_session(backend, binary, nodes, cfg, paths, &[]).await
}

/// Runs an availability probe: the same ephemeral session as Real Delay, plus
/// one request per built-in service per node through the node's own tunnel,
/// deriving a per-service verdict for each node.
///
/// Subject to the same single-session rule as Real Delay: the request queues
/// behind a running session and never spawns a second ephemeral backend.
pub async fn check_node_availability(
    backend: BackendType,
    binary: &Path,
    nodes: &[&SubscriptionNode],
    cfg: &RealDelaySettings,
    paths: &AppPaths,
    services: &[ServiceCheck],
) -> RealDelayReport {
    run_probe_session(backend, binary, nodes, cfg, paths, services).await
}

async fn run_probe_session(
    backend: BackendType,
    binary: &Path,
    nodes: &[&SubscriptionNode],
    cfg: &RealDelaySettings,
    paths: &AppPaths,
    services: &[ServiceCheck],
) -> RealDelayReport {
    let len = nodes.len();
    if len == 0 {
        return RealDelayReport {
            results: Vec::new(),
            egress: Vec::new(),
            verdicts: Vec::new(),
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
    let Some(egress_ports) = pick_free_loopback_ports(len) else {
        return RealDelayReport::failed(len, "could not allocate egress loopback ports");
    };

    let config = generator.generate(
        nodes,
        api_port,
        &cfg.test_url,
        cfg.timeout_ms,
        &egress_ports,
    );
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

    // The per-node pass (egress fetch, then one availability request per
    // service) rides the same session, after the delay collection. The pass
    // is bounded: at most MAX_CONCURRENT_NODE_PROBES futures are in flight.
    let node_pass = |config: &Value| {
        let ports = egress_inbound_ports(config);
        let timeout = Duration::from_millis(u64::from(cfg.timeout_ms));
        let services: Vec<ServiceCheck> = services.to_vec();
        run_bounded(
            MAX_CONCURRENT_NODE_PROBES,
            (0..len).map(move |i| {
                let services = services.clone();
                let port = ports.get(&i).copied();
                async move {
                    match port {
                        Some(port) => probe_node_extras(port, &services, timeout).await,
                        None => (None, HashMap::new()),
                    }
                }
            }),
        )
    };

    match backend {
        BackendType::SingBox => {
            let results = singbox_delays(api_port, len, &cfg.test_url, cfg.timeout_ms).await;
            let extras = node_pass(&config).await;
            let _ = runner.stop().await;
            drop(temp);
            finalize_report(results, extras, None)
        }
        BackendType::Xray => {
            let obs_result = observatory_delays_with_query(len, cfg.timeout_ms, || {
                query_xray_observatory(api_port)
            })
            .await;
            let extras = node_pass(&config).await;
            let _ = runner.stop().await;
            drop(temp);
            finalize_report(obs_result.results, extras, obs_result.diagnostic)
        }
        BackendType::V2ray => {
            let obs_result = observatory_delays_with_query(len, cfg.timeout_ms, || {
                query_v2ray_observatory(api_port)
            })
            .await;
            let extras = node_pass(&config).await;
            let _ = runner.stop().await;
            drop(temp);
            finalize_report(obs_result.results, extras, obs_result.diagnostic)
        }
    }
}

/// Assembles the session report from the delay results and the per-node
/// egress/verdict extras.
fn finalize_report(
    results: Vec<Option<u64>>,
    extras: Vec<(Option<EgressSample>, HashMap<String, VerdictRecord>)>,
    diagnostic: Option<String>,
) -> RealDelayReport {
    let mut egress = Vec::with_capacity(extras.len());
    let mut verdicts = Vec::with_capacity(extras.len());
    for (sample, verdict_map) in extras {
        egress.push(sample);
        verdicts.push(verdict_map);
    }
    RealDelayReport {
        results,
        egress,
        verdicts,
        diagnostic,
    }
}

/// Runs the per-node HTTP pass through the node's loopback inbound: one
/// IP-echo request to observe the egress, then one availability request per
/// service. A transport failure yields an unknown verdict, never blocked.
async fn probe_node_extras(
    port: u16,
    services: &[ServiceCheck],
    timeout: Duration,
) -> (Option<EgressSample>, HashMap<String, VerdictRecord>) {
    let Some(client) = node_proxy_client(port, timeout) else {
        return (None, HashMap::new());
    };
    let egress = fetch_egress(&client, EGRESS_ECHO_URL).await;
    let mut verdicts = HashMap::with_capacity(services.len());
    for check in services {
        verdicts.insert(check.id.to_string(), check_service(&client, check).await);
    }
    (egress, verdicts)
}

/// Builds an HTTP client that sends all traffic through the node's loopback
/// proxy inbound on `port`.
fn node_proxy_client(port: u16, timeout: Duration) -> Option<reqwest::Client> {
    let proxy = reqwest::Proxy::http(format!("http://127.0.0.1:{port}")).ok()?;
    reqwest::Client::builder()
        .proxy(proxy)
        .user_agent(BROWSER_USER_AGENT)
        .redirect(redirect_policy(REDIRECT_CAP))
        .timeout(timeout)
        .build()
        .ok()
}

/// Reads the response body in chunks, capped at `cap` bytes.
async fn fetch_capped(
    response: &mut reqwest::Response,
    cap: usize,
) -> Result<String, reqwest::Error> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > cap {
            let remaining = cap - bytes.len();
            bytes.extend_from_slice(&chunk[..remaining]);
            break;
        }
        bytes.extend_from_slice(&chunk);
        if bytes.len() >= cap {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Observes the node's egress through its tunnel. Any failure (timeout,
/// non-200, unparseable body) yields `None` and leaves any previous sample
/// intact — the caller simply does not record a new one.
async fn fetch_egress(client: &reqwest::Client, echo_url: &str) -> Option<EgressSample> {
    let mut response = client.get(echo_url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = fetch_capped(&mut response, EGRESS_BODY_CAP).await.ok()?;
    let mut address = None;
    let mut location = None;
    for line in body.lines() {
        if let Some(value) = line.strip_prefix("ip=") {
            address = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("loc=") {
            location = Some(value.trim().to_string());
        }
    }
    let address = address.filter(|a| !a.is_empty())?;
    let location = location.filter(|l| !l.is_empty())?;
    Some(EgressSample {
        address,
        location,
        checked_at: Utc::now(),
    })
}

/// Runs one service check through the node's tunnel and derives the verdict
/// from the final URL and body of the redirect chain. Transport failure maps
/// to [`AvailabilityVerdict::Unknown`], never `blocked`.
async fn check_service(client: &reqwest::Client, check: &ServiceCheck) -> VerdictRecord {
    let checked_at = Utc::now();
    let reply = async {
        let mut response = client.get(check.url).send().await?;
        let status = response.status().as_u16();
        let final_url = response.url().to_string();
        let body = fetch_capped(&mut response, AVAILABILITY_BODY_CAP).await?;
        Ok::<_, reqwest::Error>((final_url, status, body))
    };
    let result = match reply.await {
        Ok((final_url, status, body)) => derive_verdict(check, &final_url, status, &body),
        Err(_) => AvailabilityVerdict::Unknown,
    };
    VerdictRecord { result, checked_at }
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

    let futures = (0..count).map(|i| {
        let client = client.clone();
        let url = format!(
            "http://127.0.0.1:{api_port}/proxies/{tag}/delay?url={encoded_url}&timeout={timeout_ms}",
            tag = probe_tag(i),
        );
        async move { query_singbox_delay(&client, &url, request_timeout).await }
    });
    run_bounded(MAX_CONCURRENT_NODE_PROBES, futures).await
}

/// Polls per-item futures with at most `limit` in flight, returning results
/// in input order.
async fn run_bounded<I, F, T>(limit: usize, items: I) -> Vec<T>
where
    I: IntoIterator<Item = F>,
    F: Future<Output = T>,
{
    use futures_util::StreamExt;
    futures_util::stream::iter(items)
        .buffered(limit)
        .collect()
        .await
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
mod availability_tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener as TokioListener;

    /// A canned response from the stub proxy: status code and body.
    type Canned = (u16, &'static str);

    fn http_response(status: u16, body: &str) -> String {
        let reason = match status {
            200 => "OK",
            302 => "Found",
            403 => "Forbidden",
            404 => "Not Found",
            _ => "Unknown",
        };
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    /// Spawns a stub HTTP proxy that answers every request with the next canned
    /// response in sequence, repeating the last one; `None` answers nothing
    /// (the client times out). Returns its loopback port.
    async fn spawn_stub_proxy(responses: Vec<Option<Canned>>) -> u16 {
        let listener = TokioListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let cursor = std::sync::Arc::new(StdMutex::new(0usize));
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let responses = responses.clone();
                let cursor = cursor.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = socket.read(&mut buf).await;
                    let answer = {
                        let mut idx = cursor.lock().unwrap();
                        let answer = responses
                            .get(*idx)
                            .copied()
                            .flatten()
                            .or(responses.last().copied().flatten());
                        *idx = (*idx + 1).min(responses.len().saturating_sub(1));
                        answer
                    };
                    if let Some((status, body)) = answer {
                        let wire = http_response(status, body);
                        let _ = socket.write_all(wire.as_bytes()).await;
                        let _ = socket.flush().await;
                    }
                    // Dropping the socket without answering hangs the client
                    // into its timeout.
                });
            }
        });
        port
    }

    fn stub_check(url: String) -> ServiceCheck {
        ServiceCheck {
            id: "stub",
            name: "Stub",
            url: Box::leak(url.into_boxed_str()),
            blocked_url_markers: &["location=unsupported"],
            blocked_body_markers: &["not available in your country"],
        }
    }

    fn stub_client(port: u16, timeout_ms: u64) -> reqwest::Client {
        node_proxy_client(port, Duration::from_millis(timeout_ms)).unwrap()
    }

    #[tokio::test]
    async fn check_service_passes_on_unmarked_body() {
        let port = spawn_stub_proxy(vec![Some((200, "sign-in page"))]).await;
        let check = stub_check(format!("http://127.0.0.1:{port}/check"));
        let client = stub_client(port, 2000);
        let record = check_service(&client, &check).await;
        assert_eq!(record.result, AvailabilityVerdict::Available);
    }

    #[tokio::test]
    async fn check_service_redirect_chain_evaluated_at_final_url() {
        // The gate semantics live in derive_verdict: the final URL of the
        // chain, not the first hop, is what is matched.
        let check = stub_check("http://127.0.0.1:1/check".into());
        let verdict = derive_verdict(
            &check,
            "http://example.test/?location=unsupported",
            200,
            "sign-in page",
        );
        assert_eq!(verdict, AvailabilityVerdict::Blocked);
        let first_hop_only = derive_verdict(&check, "http://example.test/", 200, "sign-in page");
        assert_eq!(first_hop_only, AvailabilityVerdict::Available);
    }

    #[tokio::test]
    async fn check_service_blocked_on_refusal_body() {
        let port = spawn_stub_proxy(vec![Some((
            200,
            "Sorry, this service is not available in your country.",
        ))])
        .await;
        let check = stub_check(format!("http://127.0.0.1:{port}/check"));
        let client = stub_client(port, 2000);
        let record = check_service(&client, &check).await;
        assert_eq!(record.result, AvailabilityVerdict::Blocked);
    }

    #[tokio::test]
    async fn check_service_timeout_is_unknown() {
        // The stub accepts the connection but never answers.
        let port = spawn_stub_proxy(vec![None]).await;
        let check = stub_check(format!("http://127.0.0.1:{port}/check"));
        let client = stub_client(port, 400);
        let record = check_service(&client, &check).await;
        assert_eq!(record.result, AvailabilityVerdict::Unknown);
    }

    #[tokio::test]
    async fn check_service_non_200_final_status_is_unknown() {
        let port = spawn_stub_proxy(vec![Some((403, "blocked"))]).await;
        let check = stub_check(format!("http://127.0.0.1:{port}/check"));
        let client = stub_client(port, 2000);
        let record = check_service(&client, &check).await;
        assert_eq!(record.result, AvailabilityVerdict::Unknown);
    }

    #[tokio::test]
    async fn fetch_egress_parses_ip_and_loc() {
        let port = spawn_stub_proxy(vec![Some((200, "fl=1\nip=2001:db8::1\nloc=AT\n"))]).await;
        let client = stub_client(port, 2000);
        let sample = fetch_egress(&client, &format!("http://127.0.0.1:{port}/trace"))
            .await
            .expect("sample parsed");
        assert_eq!(sample.address, "2001:db8::1");
        assert_eq!(sample.location, "AT");
    }

    #[tokio::test]
    async fn fetch_egress_none_on_non_200() {
        let port = spawn_stub_proxy(vec![Some((404, "nope"))]).await;
        let client = stub_client(port, 2000);
        assert!(
            fetch_egress(&client, &format!("http://127.0.0.1:{port}/trace"))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn fetch_egress_none_on_malformed_body() {
        let port = spawn_stub_proxy(vec![Some((200, "hello world\nno keys here"))]).await;
        let client = stub_client(port, 2000);
        assert!(
            fetch_egress(&client, &format!("http://127.0.0.1:{port}/trace"))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn fetch_egress_none_on_timeout() {
        let port = spawn_stub_proxy(vec![None]).await;
        let client = stub_client(port, 400);
        assert!(
            fetch_egress(&client, &format!("http://127.0.0.1:{port}/trace"))
                .await
                .is_none()
        );
    }

    #[test]
    fn finalize_report_keeps_delay_results_despite_failed_egress() {
        let results = vec![Some(120), None];
        let extras = vec![(None, HashMap::new()), (None, HashMap::new())];
        let report = finalize_report(results.clone(), extras, None);
        assert_eq!(report.results, results);
        assert!(report.egress.iter().all(|e| e.is_none()));
        assert!(report.verdicts.iter().all(|v| v.is_empty()));
        assert_eq!(report.results.len(), report.egress.len());
        assert_eq!(report.results.len(), report.verdicts.len());
    }

    #[tokio::test]
    async fn availability_session_queues_behind_a_running_session() {
        // Hold the shared session lock, as a running Real Delay session would.
        let guard = REAL_DELAY_LOCK.lock().await;

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

        let task = tokio::spawn(async move {
            let binary = Path::new("/nonexistent/v2ray");
            let nodes = [&node];
            check_node_availability(
                BackendType::V2ray,
                binary,
                &nodes,
                &RealDelaySettings::default(),
                &paths,
                &[],
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            !task.is_finished(),
            "availability probe must queue while a session holds the lock"
        );

        drop(guard);
        let report = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("probe completes after the lock is released")
            .expect("no join error");
        // It proceeded to the (failing) session instead of spawning a second
        // backend concurrently.
        assert!(report.diagnostic.is_some());
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
                if let Some(idx) = parse_probe_index(&status.outbound_tag)
                    && idx < count
                {
                    results[idx] = status.delay_ms;
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

#[cfg(test)]
mod concurrency_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn bounded_pass_caps_peak_in_flight_and_keeps_order() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let futures = (0..200).map(|i| {
            let in_flight = Arc::clone(&in_flight);
            let peak = Arc::clone(&peak);
            async move {
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                i
            }
        });
        let results = run_bounded(MAX_CONCURRENT_NODE_PROBES, futures).await;
        assert_eq!(results, (0..200).collect::<Vec<_>>());
        assert!(
            peak.load(Ordering::SeqCst) <= MAX_CONCURRENT_NODE_PROBES,
            "peak in-flight {} exceeded cap {}",
            peak.load(Ordering::SeqCst),
            MAX_CONCURRENT_NODE_PROBES
        );
    }
}
