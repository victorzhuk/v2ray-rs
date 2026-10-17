use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{StreamExt, stream};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio::time::timeout;

use v2ray_rs_core::models::SubscriptionNode;

#[derive(Error, Debug)]
pub enum PingError {
    #[error("connection timed out")]
    Timeout,
    #[error("connection failed: {0}")]
    ConnectionFailed(#[from] std::io::Error),
}

const PING_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONCURRENT_PINGS: usize = 50;

pub async fn tcp_ping(addr: &str, port: u16) -> Result<Duration, PingError> {
    let target = format!("{addr}:{port}");
    let start = Instant::now();
    timeout(PING_TIMEOUT, TcpStream::connect(&target))
        .await
        .map_err(|_| PingError::Timeout)?
        .map_err(PingError::ConnectionFailed)?;
    Ok(start.elapsed())
}

/// Default process-wide admission gate: every `ping_nodes` call, concurrent
/// or not, shares these `MAX_CONCURRENT_PINGS` permits.
static PING_GATE: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_PINGS)));

/// Pings every node, bounded by the process-wide [`MAX_CONCURRENT_PINGS`]
/// permit gate shared by all callers. The returned future is boxed so it is
/// unconditionally `Send` and usable inside `oneshot_command`.
pub fn ping_nodes(
    nodes: &[SubscriptionNode],
) -> std::pin::Pin<Box<dyn Future<Output = Vec<Option<u64>>> + Send + '_>> {
    ping_nodes_with_gate(nodes, Arc::clone(&PING_GATE))
}

/// Pings every node through a caller-supplied admission gate, so concurrent
/// callers can share one probe budget. A permit is acquired only once a probe
/// future is polled: the stream bound keeps at most `MAX_CONCURRENT_PINGS`
/// futures in flight, and the gate bounds them further when it holds fewer
/// permits. Results are returned in input order.
pub fn ping_nodes_with_gate(
    nodes: &[SubscriptionNode],
    gate: Arc<Semaphore>,
) -> std::pin::Pin<Box<dyn Future<Output = Vec<Option<u64>>> + Send + '_>> {
    Box::pin(async move {
        ping_nodes_bounded(nodes, gate, |addr, port| {
            Box::pin(async move { tcp_ping(&addr, port).await })
        })
        .await
    })
}

/// The bounded ping pass with an injectable probe, so tests can observe
/// admission directly.
type ProbeFuture = Pin<Box<dyn Future<Output = Result<Duration, PingError>> + Send>>;

async fn ping_nodes_bounded<F>(
    nodes: &[SubscriptionNode],
    gate: Arc<Semaphore>,
    probe: F,
) -> Vec<Option<u64>>
where
    F: Fn(String, u16) -> ProbeFuture + Clone + Send,
{
    let mut futs = Vec::with_capacity(nodes.len());
    for node in nodes {
        let addr = node.node.address().to_string();
        let port = node.node.port();
        let gate = Arc::clone(&gate);
        let probe = probe.clone();
        futs.push(async move {
            let _permit = gate.acquire().await.ok()?;
            probe(addr, port).await.ok().map(|d| d.as_millis() as u64)
        });
    }
    stream::iter(futs)
        .buffered(MAX_CONCURRENT_PINGS)
        .collect()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;
    use v2ray_rs_core::models::{ProxyNode, TransportSettings, VlessConfig};

    fn node() -> SubscriptionNode {
        SubscriptionNode {
            id: Uuid::new_v4(),
            node: ProxyNode::Vless(VlessConfig {
                address: "127.0.0.1".into(),
                port: 9,
                uuid: "test-uuid".into(),
                encryption: None,
                flow: None,
                transport: TransportSettings::Tcp,
                tls: None,
                remark: None,
            }),
            enabled: true,
            last_latency_ms: None,
            last_real_delay_ms: None,
        }
    }

    #[tokio::test]
    async fn concurrent_ping_passes_share_admission_cap() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let probe = {
            let in_flight = Arc::clone(&in_flight);
            let peak = Arc::clone(&peak);
            move |_: String, _: u16| {
                let in_flight = Arc::clone(&in_flight);
                let peak = Arc::clone(&peak);
                let f: ProbeFuture = Box::pin(async move {
                    let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    Ok(Duration::from_millis(1))
                });
                f
            }
        };

        let nodes: Vec<_> = (0..80).map(|_| node()).collect();
        let gate = Arc::new(Semaphore::new(MAX_CONCURRENT_PINGS));
        let (first, second) = tokio::join!(
            ping_nodes_bounded(&nodes, Arc::clone(&gate), probe.clone()),
            ping_nodes_bounded(&nodes, Arc::clone(&gate), probe)
        );

        assert_eq!(first.len(), 80);
        assert_eq!(second.len(), 80);
        assert!(first.iter().all(|r| r.is_some()));
        assert!(second.iter().all(|r| r.is_some()));
        assert!(
            peak.load(Ordering::SeqCst) <= MAX_CONCURRENT_PINGS,
            "peak in-flight {} exceeded cap {}",
            peak.load(Ordering::SeqCst),
            MAX_CONCURRENT_PINGS
        );
    }
}
