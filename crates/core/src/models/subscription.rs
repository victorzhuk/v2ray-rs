use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ImportedProfile, ProxyNode};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub name: String,
    pub source: SubscriptionSource,
    pub nodes: Vec<SubscriptionNode>,
    pub last_updated: Option<DateTime<Utc>>,
    pub auto_update_interval_secs: Option<u64>,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_profile: Option<ImportedProfile>,
    #[serde(default = "default_true")]
    pub use_imported_profile: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SubscriptionSource {
    Url { url: String },
    File { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionNode {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub node: ProxyNode,
    pub enabled: bool,
    #[serde(skip, default)]
    pub last_latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_real_delay_ms: Option<u64>,
}

impl Subscription {
    pub fn new_from_url(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            source: SubscriptionSource::Url { url: url.into() },
            nodes: Vec::new(),
            last_updated: None,
            auto_update_interval_secs: Some(86400),
            enabled: true,
            imported_profile: None,
            use_imported_profile: true,
        }
    }

    pub fn new_from_file(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            source: SubscriptionSource::File { path: path.into() },
            nodes: Vec::new(),
            last_updated: None,
            auto_update_interval_secs: None,
            enabled: true,
            imported_profile: None,
            use_imported_profile: true,
        }
    }

    pub fn enabled_nodes(&self) -> impl Iterator<Item = &ProxyNode> {
        self.nodes.iter().filter(|n| n.enabled).map(|n| &n.node)
    }

    #[must_use]
    pub fn has_enabled_nodes(&self) -> bool {
        self.enabled && self.nodes.iter().any(|n| n.enabled)
    }

    #[must_use]
    pub fn runtime_state_eq(&self, other: &Self) -> bool {
        self.enabled == other.enabled
            && self.nodes.len() == other.nodes.len()
            && self
                .nodes
                .iter()
                .zip(&other.nodes)
                .all(|(lhs, rhs)| lhs.runtime_state_eq(rhs))
    }
}

impl SubscriptionNode {
    pub fn new(node: ProxyNode) -> Self {
        Self {
            id: Uuid::new_v4(),
            node,
            enabled: true,
            last_latency_ms: None,
            last_real_delay_ms: None,
        }
    }

    pub fn with_id(id: Uuid, node: ProxyNode, enabled: bool) -> Self {
        Self {
            id,
            node,
            enabled,
            last_latency_ms: None,
            last_real_delay_ms: None,
        }
    }

    #[must_use]
    pub fn runtime_state_eq(&self, other: &Self) -> bool {
        self.id == other.id && self.node == other.node && self.enabled == other.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{TransportSettings, VlessConfig};

    fn vless_node() -> ProxyNode {
        ProxyNode::Vless(VlessConfig {
            address: "example.com".into(),
            port: 443,
            uuid: "test-uuid".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: None,
        })
    }

    #[test]
    fn new_subscription_node_has_no_real_delay() {
        let node = SubscriptionNode::new(vless_node());
        assert_eq!(node.last_real_delay_ms, None);
    }

    #[test]
    fn real_delay_round_trips_json() {
        let mut node = SubscriptionNode::new(vless_node());
        node.last_real_delay_ms = Some(412);
        let json = serde_json::to_string(&node).unwrap();
        let deserialized: SubscriptionNode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.last_real_delay_ms, Some(412));
    }

    #[test]
    fn missing_real_delay_field_deserializes_to_none() {
        let json = serde_json::json!({
            "id": Uuid::new_v4(),
            "node": vless_node(),
            "enabled": true
        });
        let node: SubscriptionNode = serde_json::from_value(json).unwrap();
        assert_eq!(node.last_real_delay_ms, None);
    }

    #[test]
    fn legacy_subscription_without_profile_fields_deserializes() {
        let json = serde_json::json!({
            "id": Uuid::new_v4(),
            "name": "Legacy",
            "source": { "type": "url", "url": "https://example.com/sub" },
            "nodes": [],
            "last_updated": null,
            "auto_update_interval_secs": 86400,
            "enabled": true,
        });
        let sub: Subscription = serde_json::from_value(json).unwrap();
        assert_eq!(sub.imported_profile, None);
        assert!(sub.use_imported_profile);
    }

    #[test]
    fn new_from_url_defaults_use_imported_profile_true() {
        let sub = Subscription::new_from_url("Test", "https://example.com/sub");
        assert!(sub.use_imported_profile);
        assert!(sub.imported_profile.is_none());
    }
}
