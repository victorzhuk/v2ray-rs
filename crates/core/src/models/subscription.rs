use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ProxyNode;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub name: String,
    pub source: SubscriptionSource,
    pub nodes: Vec<SubscriptionNode>,
    pub last_updated: Option<DateTime<Utc>>,
    pub auto_update_interval_secs: Option<u64>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SubscriptionSource {
    Url { url: String },
    File { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionNode {
    pub node: ProxyNode,
    pub enabled: bool,
    #[serde(skip_serializing, default)]
    pub last_latency_ms: Option<u64>,
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
        }
    }

    pub fn enabled_nodes(&self) -> impl Iterator<Item = &ProxyNode> {
        self.nodes.iter().filter(|n| n.enabled).map(|n| &n.node)
    }

    pub fn has_enabled_nodes(&self) -> bool {
        self.enabled && self.nodes.iter().any(|n| n.enabled)
    }
}
