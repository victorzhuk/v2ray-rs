use std::sync::Arc;
use std::time::{Duration, Instant};

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

pub async fn ping_nodes(nodes: &[SubscriptionNode]) -> Vec<Option<u64>> {
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_PINGS));
    let handles: Vec<_> = nodes
        .iter()
        .map(|node| {
            let addr = node.node.address().to_string();
            let port = node.node.port();
            let permit = Arc::clone(&semaphore);
            tokio::spawn(async move {
                let _permit = permit.acquire().await.ok()?;
                tcp_ping(&addr, port)
                    .await
                    .ok()
                    .map(|d| d.as_millis() as u64)
            })
        })
        .collect();

    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        results.push(handle.await.ok().flatten());
    }
    results
}
