use std::collections::HashMap;
use std::mem::{Discriminant, discriminant};
use std::time::Duration;

use chrono::Utc;
use thiserror::Error;
use v2ray_rs_core::models::{
    ImportedProfile, ProxyNode, Subscription, SubscriptionNode, SubscriptionSource,
};

#[cfg(any(test, feature = "test-utils"))]
use crate::fetch::fetch_with_client_allow_private;
use crate::fetch::{FetchError, fetch_from_file, fetch_with_client};
use crate::json_import::parse_json_subscription;
use crate::parser::parse_subscription_uris;
use std::future::Future;

const DEFAULT_MAX_RETRIES: u32 = 3;

/// Hard ceiling on parsed proxy nodes per subscription import, enforced
/// before any reconciliation or persistence. Keeps a compact URI list within
/// the 10 MiB payload limit from materializing an unbounded widget tree.
pub const MAX_SUBSCRIPTION_NODES: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFailure {
    pub uri: String,
    pub error: String,
}

#[derive(Debug, Clone)]
pub struct UpdateResult {
    pub added: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub parse_failures: Vec<ParseFailure>,
    pub profile: Option<ImportedProfile>,
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("subscription contained no valid proxy URIs")]
    InvalidContent { failures: Vec<ParseFailure> },
    #[error(
        "subscription exceeds the node limit: {parsed} parsed nodes (maximum {})",
        MAX_SUBSCRIPTION_NODES
    )]
    TooManyNodes { parsed: usize },
}

pub fn reconcile_nodes(
    old_nodes: &[SubscriptionNode],
    new_parsed: Vec<ProxyNode>,
) -> Vec<SubscriptionNode> {
    reconcile_with_counts(old_nodes, new_parsed).0
}

type NodeKey = (String, u16, Discriminant<ProxyNode>);

fn node_key(node: &ProxyNode) -> NodeKey {
    (node.address().to_owned(), node.port(), discriminant(node))
}

/// Connection equality that ignores the display remark, so a provider renaming
/// a node does not count as a connection change.
pub fn same_connection(left: &ProxyNode, right: &ProxyNode) -> bool {
    fn without_remark(node: &ProxyNode) -> ProxyNode {
        let mut node = node.clone();
        match &mut node {
            ProxyNode::Vless(c) => c.remark = None,
            ProxyNode::Vmess(c) => c.remark = None,
            ProxyNode::Shadowsocks(c) => c.remark = None,
            ProxyNode::Trojan(c) => c.remark = None,
        }
        node
    }
    without_remark(left) == without_remark(right)
}

pub fn reconcile_with_counts(
    old_nodes: &[SubscriptionNode],
    new_parsed: Vec<ProxyNode>,
) -> (Vec<SubscriptionNode>, UpdateResult) {
    let mut index: HashMap<NodeKey, Vec<usize>> = HashMap::new();
    for (idx, old) in old_nodes.iter().enumerate() {
        index.entry(node_key(&old.node)).or_default().push(idx);
    }

    let mut added = 0;
    let mut unchanged = 0;
    let mut matched_old = vec![false; old_nodes.len()];
    let mut result = Vec::new();

    let mut matches = vec![None; new_parsed.len()];
    for (position, new_node) in new_parsed.iter().enumerate() {
        let key = node_key(new_node);
        if let Some(idx) = index
            .get(&key)
            .into_iter()
            .flatten()
            .copied()
            .find(|&idx| !matched_old[idx] && same_connection(&old_nodes[idx].node, new_node))
        {
            matches[position] = Some(idx);
            matched_old[idx] = true;
        }
    }
    let mut remaining_counts = HashMap::new();
    for (position, node) in new_parsed.iter().enumerate() {
        if matches[position].is_none() {
            *remaining_counts.entry(node_key(node)).or_insert(0usize) += 1;
        }
    }
    for (position, new_node) in new_parsed.iter().enumerate() {
        if matches[position].is_some() {
            continue;
        }
        let key = node_key(new_node);
        if remaining_counts.get(&key) != Some(&1) {
            continue;
        }
        let mut old = index
            .get(&key)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&idx| !matched_old[idx]);
        if let Some(idx) = old.next()
            && old.next().is_none()
        {
            matches[position] = Some(idx);
            matched_old[idx] = true;
        }
    }
    for (new_node, matched) in new_parsed.into_iter().zip(matches) {
        if let Some(idx) = matched {
            unchanged += 1;
            let old = &old_nodes[idx];
            let same = same_connection(&old.node, &new_node);
            let mut node = SubscriptionNode::with_id(old.id, new_node, old.enabled);
            if same {
                node.last_latency_ms = old.last_latency_ms;
                node.last_real_delay_ms = old.last_real_delay_ms;
            }
            result.push(node);
        } else {
            added += 1;
            result.push(SubscriptionNode::new(new_node));
        }
    }

    let removed = old_nodes.len().saturating_sub(unchanged);

    let update_result = UpdateResult {
        added,
        removed,
        unchanged,
        parse_failures: Vec::new(),
        profile: None,
    };

    (result, update_result)
}

pub async fn fetch_with_retry(
    client: &reqwest::Client,
    url: &str,
    max_retries: u32,
) -> Result<String, FetchError> {
    fetch_with_retry_impl(
        || fetch_with_client(client, url),
        tokio::time::sleep,
        max_retries,
    )
    .await
}

/// Loopback-only fixtures; pairs with `fetch_with_client_allow_private` so
/// tests exercise the real retry loop against local HTTP servers.
#[cfg(any(test, feature = "test-utils"))]
pub async fn fetch_with_retry_allow_private(
    client: &reqwest::Client,
    url: &str,
    max_retries: u32,
) -> Result<String, FetchError> {
    fetch_with_retry_impl(
        || fetch_with_client_allow_private(client, url),
        tokio::time::sleep,
        max_retries,
    )
    .await
}

async fn fetch_with_retry_impl<F, Fut, S, Slp>(
    fetch_fn: F,
    sleep_fn: S,
    max_retries: u32,
) -> Result<String, FetchError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<String, FetchError>>,
    S: Fn(Duration) -> Slp,
    Slp: Future<Output = ()>,
{
    let mut attempt = 0;
    loop {
        match fetch_fn().await {
            Ok(content) => return Ok(content),
            Err(e) => {
                if !e.is_transient() {
                    return Err(e);
                }
                if attempt >= max_retries {
                    return Err(e);
                }
                let delay = Duration::from_secs(1 << attempt);
                sleep_fn(delay).await;
                attempt += 1;
            }
        }
    }
}

pub async fn update_subscription(
    client: &reqwest::Client,
    subscription: &mut Subscription,
) -> Result<UpdateResult, UpdateError> {
    update_subscription_impl(client, subscription, FetchMode::Enforced).await
}

/// Loopback-only fixtures; routes the fetch through the permissive
/// `fetch_with_client_allow_private` while keeping parse, node-cap, and
/// reconciliation behavior identical to production.
#[cfg(any(test, feature = "test-utils"))]
pub async fn update_subscription_allow_private(
    client: &reqwest::Client,
    subscription: &mut Subscription,
) -> Result<UpdateResult, UpdateError> {
    update_subscription_impl(client, subscription, FetchMode::AllowPrivate).await
}

/// Chooses the fetch entry point for one subscription update. Parse,
/// node-cap, and reconciliation behavior are identical for every mode.
#[derive(Clone, Copy)]
enum FetchMode {
    Enforced,
    #[cfg(any(test, feature = "test-utils"))]
    AllowPrivate,
}

async fn update_subscription_impl(
    client: &reqwest::Client,
    subscription: &mut Subscription,
    mode: FetchMode,
) -> Result<UpdateResult, UpdateError> {
    let raw_content = match &subscription.source {
        SubscriptionSource::Url { url } => {
            let url = url.clone();
            match mode {
                FetchMode::Enforced => fetch_with_retry(client, &url, DEFAULT_MAX_RETRIES).await?,
                #[cfg(any(test, feature = "test-utils"))]
                FetchMode::AllowPrivate => {
                    fetch_with_retry_allow_private(client, &url, DEFAULT_MAX_RETRIES).await?
                }
            }
        }
        SubscriptionSource::File { path } => {
            let path = path.clone();
            tokio::task::spawn_blocking(move || fetch_from_file(&path))
                .await
                .map_err(|e| FetchError::FileError(std::io::Error::other(e.to_string())))??
        }
    };

    // A 10 MiB subscription can expand into tens of thousands of URIs; both
    // base64 decoding and URI parsing are CPU-bound and must not stall the
    // async runtime that other subscriptions share.
    let (profile, import) =
        tokio::task::spawn_blocking(move || match parse_json_subscription(&raw_content) {
            Some(j) => (j.profile, j.result),
            None => {
                let uris = crate::fetch::decode_subscription_content(&raw_content);
                (None, parse_subscription_uris(&uris))
            }
        })
        .await
        .map_err(|e| UpdateError::InvalidContent {
            failures: vec![ParseFailure {
                uri: "<subscription>".to_string(),
                error: format!("parse task failed: {e}"),
            }],
        })?;

    let parse_failures: Vec<ParseFailure> = import
        .errors
        .into_iter()
        .map(|(uri, error)| ParseFailure {
            uri,
            error: error.to_string(),
        })
        .collect();

    if import.nodes.len() > MAX_SUBSCRIPTION_NODES {
        return Err(UpdateError::TooManyNodes {
            parsed: import.nodes.len(),
        });
    }

    if import.nodes.is_empty() {
        return Err(UpdateError::InvalidContent {
            failures: parse_failures,
        });
    }

    let parsed_nodes = import.nodes.into_iter().map(|node| node.node).collect();
    let (new_nodes, result) = reconcile_with_counts(&subscription.nodes, parsed_nodes);

    subscription.nodes = new_nodes;
    subscription.last_updated = Some(Utc::now());
    subscription.imported_profile = profile.clone();

    Ok(UpdateResult {
        parse_failures,
        profile,
        ..result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Once;
    use std::sync::atomic::{AtomicU32, Ordering};
    use uuid::Uuid;
    use v2ray_rs_core::models::{ShadowsocksConfig, TransportSettings, VlessConfig, VmessConfig};

    fn test_client() -> reqwest::Client {
        static RUSTLS_PROVIDER: Once = Once::new();
        RUSTLS_PROVIDER.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
        reqwest::Client::new()
    }

    fn vless_node(addr: &str, port: u16) -> ProxyNode {
        ProxyNode::Vless(VlessConfig {
            address: addr.to_owned(),
            port,
            uuid: "test-uuid".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: None,
        })
    }

    fn vmess_node(addr: &str, port: u16) -> ProxyNode {
        ProxyNode::Vmess(VmessConfig {
            address: addr.to_owned(),
            port,
            uuid: "test-uuid".into(),
            alter_id: 0,
            security: "auto".into(),
            transport: TransportSettings::Tcp,
            tls: None,
            remark: None,
        })
    }

    fn ss_node(addr: &str, port: u16) -> ProxyNode {
        ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: addr.to_owned(),
            port,
            method: "aes-256-gcm".into(),
            password: "pass".into(),
            remark: None,
        })
    }

    #[test]
    fn test_reconcile_preserves_enabled() {
        let mut old_node = SubscriptionNode::new(vless_node("example.com", 443));
        old_node.enabled = false;
        let old = vec![old_node];

        let new_parsed = vec![vless_node("example.com", 443)];

        let result = reconcile_nodes(&old, new_parsed);

        assert_eq!(result.len(), 1);
        assert!(!result[0].enabled);
    }

    #[test]
    fn test_reconcile_adds_new_nodes() {
        let old = vec![SubscriptionNode::new(vless_node("a.com", 443))];

        let new_parsed = vec![vless_node("a.com", 443), vless_node("b.com", 443)];

        let result = reconcile_nodes(&old, new_parsed);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].node.address(), "a.com");
        assert_eq!(result[1].node.address(), "b.com");
        assert!(result[1].enabled);
    }

    #[test]
    fn test_reconcile_removes_missing() {
        let old = vec![
            SubscriptionNode::new(vless_node("a.com", 443)),
            SubscriptionNode::new(vless_node("b.com", 443)),
        ];

        let new_parsed = vec![vless_node("a.com", 443)];

        let result = reconcile_nodes(&old, new_parsed);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].node.address(), "a.com");
    }

    #[test]
    fn test_reconcile_all_replaced() {
        let mut old_node = SubscriptionNode::new(vless_node("a.com", 443));
        old_node.enabled = false;
        let old = vec![old_node];

        let new_parsed = vec![vless_node("b.com", 443)];

        let result = reconcile_nodes(&old, new_parsed);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].node.address(), "b.com");
        assert!(result[0].enabled);
    }

    #[test]
    fn test_reconcile_empty_old() {
        let old = vec![];
        let new_parsed = vec![vless_node("a.com", 443), vless_node("b.com", 443)];

        let result = reconcile_nodes(&old, new_parsed);

        assert_eq!(result.len(), 2);
        assert!(result[0].enabled);
        assert!(result[1].enabled);
    }

    #[test]
    fn test_reconcile_empty_new() {
        let old = vec![SubscriptionNode::new(vless_node("a.com", 443))];

        let new_parsed = vec![];

        let result = reconcile_nodes(&old, new_parsed);

        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_update_result_counts() {
        let old = vec![
            SubscriptionNode::new(vless_node("a.com", 443)),
            SubscriptionNode::with_id(Uuid::new_v4(), vmess_node("b.com", 8443), false),
        ];

        let new_parsed = vec![vless_node("a.com", 443), ss_node("c.com", 8388)];

        let (_nodes, result) = reconcile_with_counts(&old, new_parsed);

        assert_eq!(result.added, 1);
        assert_eq!(result.removed, 1);
        assert_eq!(result.unchanged, 1);
    }

    #[test]
    fn test_reconcile_preserves_real_delay_sample() {
        let mut old_node = SubscriptionNode::new(vless_node("example.com", 443));
        old_node.last_latency_ms = Some(38);
        old_node.last_real_delay_ms = Some(412);
        let old = vec![old_node];

        let new_parsed = vec![vless_node("example.com", 443)];
        let result = reconcile_nodes(&old, new_parsed);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].last_latency_ms, Some(38));
        assert_eq!(result[0].last_real_delay_ms, Some(412));
    }

    #[test]
    fn reordered_endpoint_duplicates_keep_preferences() {
        let first = vless_node("example.com", 443);
        let mut second = first.clone();
        if let ProxyNode::Vless(c) = &mut second {
            c.uuid = "second".into();
        }
        let old = vec![
            SubscriptionNode::with_id(Uuid::new_v4(), first.clone(), false),
            SubscriptionNode::with_id(Uuid::new_v4(), second.clone(), true),
        ];
        let result = reconcile_nodes(&old, vec![second, first]);
        assert_eq!(result[0].id, old[1].id);
        assert_eq!(result[1].id, old[0].id);
        assert!(result[0].enabled);
        assert!(!result[1].enabled);
    }

    #[test]
    fn changed_connection_invalidates_measurements() {
        let mut old = SubscriptionNode::new(vless_node("example.com", 443));
        old.last_real_delay_ms = Some(100);
        old.last_latency_ms = Some(20);
        let mut changed = old.node.clone();
        if let ProxyNode::Vless(c) = &mut changed {
            c.uuid = "changed".into();
        }
        let result = reconcile_nodes(&[old.clone()], vec![changed]);
        assert_eq!(result[0].id, old.id);
        assert_eq!(result[0].last_real_delay_ms, None);
        assert_eq!(result[0].last_latency_ms, None);
    }

    #[test]
    fn test_reconcile_preserves_stable_ids_for_duplicates() {
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();
        let old = vec![
            SubscriptionNode::with_id(first_id, vless_node("dup.com", 443), false),
            SubscriptionNode::with_id(second_id, vless_node("dup.com", 443), true),
        ];

        let result = reconcile_nodes(
            &old,
            vec![vless_node("dup.com", 443), vless_node("dup.com", 443)],
        );

        assert_eq!(result[0].id, first_id);
        assert_eq!(result[1].id, second_id);
        assert!(!result[0].enabled);
        assert!(result[1].enabled);
    }

    #[tokio::test]
    async fn test_update_subscription_json_bundle_sets_profile() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("bundle.json");
        std::fs::write(
            &file_path,
            include_str!("../tests/fixtures/bundle_sample.json"),
        )
        .unwrap();

        let mut subscription =
            Subscription::new_from_file("Bundle", file_path.to_string_lossy().into_owned());
        let client = test_client();

        let result = update_subscription(&client, &mut subscription)
            .await
            .unwrap();

        assert_eq!(subscription.nodes.len(), 24);
        assert!(subscription.imported_profile.is_some());
        assert!(result.profile.is_some());
        assert_eq!(result.profile.unwrap().rules.len(), 4);
    }

    #[tokio::test]
    async fn test_update_subscription_uri_list_leaves_profile_none() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("subscription.txt");
        std::fs::write(&file_path, "vless://uuid@a.com:443#A").unwrap();

        let mut subscription =
            Subscription::new_from_file("Test", file_path.to_string_lossy().into_owned());
        let client = test_client();

        let result = update_subscription(&client, &mut subscription)
            .await
            .unwrap();

        assert!(subscription.imported_profile.is_none());
        assert!(result.profile.is_none());
    }

    #[tokio::test]
    async fn test_update_subscription_reports_partial_parse_failures() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("subscription.txt");
        let content = "vless://uuid@a.com:443#A\ninvalid://uri\nvless://uuid@b.com:443#B";
        std::fs::write(&file_path, content).unwrap();

        let mut subscription =
            Subscription::new_from_file("Test", file_path.to_string_lossy().into_owned());
        let client = test_client();

        let result = update_subscription(&client, &mut subscription)
            .await
            .unwrap();

        assert_eq!(subscription.nodes.len(), 2);
        assert_eq!(result.parse_failures.len(), 1);
        assert_eq!(result.parse_failures[0].uri, "invalid://uri");
    }

    #[tokio::test]
    async fn test_update_subscription_rejects_invalid_content() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("subscription.txt");
        std::fs::write(&file_path, "invalid://uri\nbroken").unwrap();

        let mut subscription =
            Subscription::new_from_file("Test", file_path.to_string_lossy().into_owned());
        let client = test_client();

        let error = update_subscription(&client, &mut subscription)
            .await
            .unwrap_err();

        match error {
            UpdateError::InvalidContent { failures } => {
                assert_eq!(failures.len(), 2);
            }
            other => panic!("expected InvalidContent, got {other:?}"),
        }
        assert!(subscription.nodes.is_empty());
    }

    /// Answers with the address of a live loopback listener so the test can
    /// prove the private answer is refused inside the resolver, before any
    /// connect attempt.
    struct RebindResolver {
        calls: Arc<AtomicU32>,
        addr: std::net::SocketAddr,
    }

    impl reqwest::dns::Resolve for RebindResolver {
        fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(std::future::ready(Ok(Box::new(
                vec![self.addr].into_iter(),
            ) as reqwest::dns::Addrs)))
        }
    }

    #[tokio::test]
    async fn rebinding_dns_answer_refused_without_retries() {
        let _ = test_client();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let calls = Arc::new(AtomicU32::new(0));
        let client = reqwest::Client::builder()
            .dns_resolver(crate::fetch::PublicTargetsResolver::with_inner(Arc::new(
                RebindResolver {
                    calls: calls.clone(),
                    addr: listener.local_addr().unwrap(),
                },
            )))
            .no_proxy()
            .build()
            .unwrap();

        let result = fetch_with_retry_impl(
            || fetch_with_client(&client, "http://rebind.example.internal/sub"),
            |_: Duration| async {},
            3,
        )
        .await;

        let err = result.unwrap_err();
        assert!(matches!(err, FetchError::InvalidUrl(_)), "{err:?}");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a DNS-answer refusal is non-transient and must not retry"
        );
        // The resolver refused the address, so nothing ever dialed the live
        // listener: its backlog must stay empty.
        let accept = listener.accept().map(|_| ());
        assert_eq!(
            accept.unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "the private DNS answer must never open a connection"
        );
    }

    #[tokio::test]
    async fn terminal_short_circuits() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();
        let fetch_fn = move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { Err(FetchError::InvalidUrl("bad".into())) }
        };
        let sleep_fn = |_: Duration| async {};

        let result = fetch_with_retry_impl(fetch_fn, sleep_fn, 3).await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FetchError::InvalidUrl(_)));
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn transient_retries_then_succeeds() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();
        let fetch_fn = move || {
            let n = count.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err(FetchError::Timeout)
                } else {
                    Ok("hello".into())
                }
            }
        };
        let sleep_fn = |_: Duration| async {};

        let result = fetch_with_retry_impl(fetch_fn, sleep_fn, 5).await;

        assert_eq!(result.unwrap(), "hello");
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn transient_exhausts_retries() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();
        let fetch_fn = move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { Err(FetchError::Timeout) }
        };
        let sleep_fn = |_: Duration| async {};

        let result = fetch_with_retry_impl(fetch_fn, sleep_fn, 3).await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FetchError::Timeout));
        assert_eq!(call_count.load(Ordering::SeqCst), 4);
    }
}
