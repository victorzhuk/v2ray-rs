use std::collections::HashMap;
use std::mem::{Discriminant, discriminant};
use std::time::Duration;

use chrono::Utc;
use thiserror::Error;
use v2ray_rs_core::models::{
    ImportedProfile, ProxyNode, Subscription, SubscriptionNode, SubscriptionSource,
};

use crate::fetch::{FetchError, fetch_from_file, fetch_with_client};
use crate::json_import::parse_json_subscription;
use crate::parser::parse_subscription_uris;
use std::future::Future;

const DEFAULT_MAX_RETRIES: u32 = 3;

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

    for new_node in new_parsed {
        let key = node_key(&new_node);
        let matched = index
            .get(&key)
            .and_then(|indices| indices.iter().copied().find(|&idx| !matched_old[idx]));

        if let Some(idx) = matched {
            matched_old[idx] = true;
            unchanged += 1;
            let old = &old_nodes[idx];
            let mut subscription_node = SubscriptionNode::with_id(old.id, new_node, old.enabled);
            subscription_node.last_latency_ms = old.last_latency_ms;
            subscription_node.last_real_delay_ms = old.last_real_delay_ms;
            result.push(subscription_node);
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
    let raw_content = match &subscription.source {
        SubscriptionSource::Url { url } => {
            fetch_with_retry(client, url, DEFAULT_MAX_RETRIES).await?
        }
        SubscriptionSource::File { path } => fetch_from_file(path)?,
    };

    let profile;
    let import = match parse_json_subscription(&raw_content) {
        Some(j) => {
            profile = j.profile;
            j.result
        }
        None => {
            let uris = crate::fetch::decode_subscription_content(&raw_content);
            profile = None;
            parse_subscription_uris(&uris)
        }
    };

    let parse_failures: Vec<ParseFailure> = import
        .errors
        .into_iter()
        .map(|(uri, error)| ParseFailure {
            uri,
            error: error.to_string(),
        })
        .collect();

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
        assert_eq!(result.profile.unwrap().rules.len(), 6);
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
