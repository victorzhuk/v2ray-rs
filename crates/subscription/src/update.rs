use std::mem::discriminant;
use std::time::Duration;

use chrono::Utc;
use uuid::Uuid;
use v2ray_rs_core::models::{ProxyNode, Subscription, SubscriptionNode, SubscriptionSource};

use crate::fetch::{fetch_from_file, fetch_with_client, FetchError};
use crate::parser::parse_uri;

#[derive(Debug, Clone)]
pub struct UpdateResult {
    pub added: usize,
    pub removed: usize,
    pub unchanged: usize,
}

#[derive(Debug, Clone)]
pub enum UpdateEvent {
    Started { subscription_id: Uuid },
    Success { subscription_id: Uuid, result: UpdateResult },
    Failed { subscription_id: Uuid, error: String },
    Retrying { subscription_id: Uuid, attempt: u32 },
}

pub fn reconcile_nodes(
    old_nodes: &[SubscriptionNode],
    new_parsed: Vec<ProxyNode>,
) -> Vec<SubscriptionNode> {
    let mut result = Vec::new();

    for new_node in new_parsed {
        let matched = old_nodes.iter().find(|old| {
            let old_node = &old.node;
            old_node.address() == new_node.address()
                && old_node.port() == new_node.port()
                && discriminant(old_node) == discriminant(&new_node)
        });

        let enabled = matched.map(|m| m.enabled).unwrap_or(true);
        result.push(SubscriptionNode {
            node: new_node,
            enabled,
            last_latency_ms: None,
        });
    }

    result
}

pub fn reconcile_with_counts(
    old_nodes: &[SubscriptionNode],
    new_parsed: Vec<ProxyNode>,
) -> (Vec<SubscriptionNode>, UpdateResult) {
    let mut added = 0;
    let mut unchanged = 0;

    let new_nodes = reconcile_nodes(old_nodes, new_parsed.clone());

    for node in &new_nodes {
        let was_present = old_nodes.iter().any(|old| {
            old.node.address() == node.node.address()
                && old.node.port() == node.node.port()
                && discriminant(&old.node) == discriminant(&node.node)
        });

        if was_present {
            unchanged += 1;
        } else {
            added += 1;
        }
    }

    let removed = old_nodes.len().saturating_sub(unchanged);

    let result = UpdateResult {
        added,
        removed,
        unchanged,
    };

    (new_nodes, result)
}

pub async fn fetch_with_retry(
    client: &reqwest::Client,
    url: &str,
    max_retries: u32,
) -> Result<String, FetchError> {
    let mut last_error = None;

    for attempt in 0..=max_retries {
        match fetch_with_client(client, url).await {
            Ok(content) => return Ok(content),
            Err(e) => {
                last_error = Some(e);
                if attempt < max_retries {
                    let delay = Duration::from_secs(1 << attempt);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    Err(last_error.unwrap())
}

pub async fn update_subscription(
    client: &reqwest::Client,
    subscription: &mut Subscription,
) -> Result<UpdateResult, FetchError> {
    let raw_content = match &subscription.source {
        SubscriptionSource::Url { url } => fetch_with_retry(client, url, 3).await?,
        SubscriptionSource::File { path } => fetch_from_file(path)?,
    };

    let uris = crate::fetch::decode_subscription_content(&raw_content);

    let mut parsed_nodes = Vec::new();
    for uri in uris {
        if let Ok(node) = parse_uri(&uri) {
            parsed_nodes.push(node);
        }
    }

    let (new_nodes, result) = reconcile_with_counts(&subscription.nodes, parsed_nodes);

    subscription.nodes = new_nodes;
    subscription.last_updated = Some(Utc::now());

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use v2ray_rs_core::models::{ShadowsocksConfig, VlessConfig, VmessConfig, TransportSettings};

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
        let old = vec![SubscriptionNode {
            node: vless_node("example.com", 443),
            enabled: false,
            last_latency_ms: None,
        }];

        let new_parsed = vec![vless_node("example.com", 443)];

        let result = reconcile_nodes(&old, new_parsed);

        assert_eq!(result.len(), 1);
        assert!(!result[0].enabled);
    }

    #[test]
    fn test_reconcile_adds_new_nodes() {
        let old = vec![SubscriptionNode {
            node: vless_node("a.com", 443),
            enabled: true,
            last_latency_ms: None,
        }];

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
            SubscriptionNode {
                node: vless_node("a.com", 443),
                enabled: true,
            last_latency_ms: None,
            },
            SubscriptionNode {
                node: vless_node("b.com", 443),
                enabled: true,
            last_latency_ms: None,
            },
        ];

        let new_parsed = vec![vless_node("a.com", 443)];

        let result = reconcile_nodes(&old, new_parsed);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].node.address(), "a.com");
    }

    #[test]
    fn test_reconcile_all_replaced() {
        let old = vec![SubscriptionNode {
            node: vless_node("a.com", 443),
            enabled: false,
            last_latency_ms: None,
        }];

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
        let old = vec![SubscriptionNode {
            node: vless_node("a.com", 443),
            enabled: true,
            last_latency_ms: None,
        }];

        let new_parsed = vec![];

        let result = reconcile_nodes(&old, new_parsed);

        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_update_result_counts() {
        let old = vec![
            SubscriptionNode {
                node: vless_node("a.com", 443),
                enabled: true,
            last_latency_ms: None,
            },
            SubscriptionNode {
                node: vmess_node("b.com", 8443),
                enabled: false,
            last_latency_ms: None,
            },
        ];

        let new_parsed = vec![
            vless_node("a.com", 443),
            ss_node("c.com", 8388),
        ];

        let (_nodes, result) = reconcile_with_counts(&old, new_parsed);

        assert_eq!(result.added, 1);
        assert_eq!(result.removed, 1);
        assert_eq!(result.unchanged, 1);
    }
}
