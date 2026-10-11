use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::geo_probe::AvailabilityVerdict;
use crate::models::BackendType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ConnectionNodeRef {
    Subscription {
        subscription_id: Uuid,
        node_id: Uuid,
    },
    Manual {
        node_id: Uuid,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AutoResolveStrategy {
    #[default]
    ListOrder,
    LowestLatency,
    Random,
    LastSuccessful,
}

impl fmt::Display for AutoResolveStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ListOrder => f.write_str("list order"),
            Self::LowestLatency => f.write_str("lowest latency"),
            Self::Random => f.write_str("random"),
            Self::LastSuccessful => f.write_str("last successful"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionMetadata {
    pub node_ref: ConnectionNodeRef,
    pub source: String,
    pub source_id: String,
    pub node_name: String,
    pub node_address: String,
    pub node_port: u16,
    pub backend: BackendType,
    pub strategy: AutoResolveStrategy,
    pub latency_ms: Option<u64>,
    pub connected_since: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastSuccessMetadata {
    pub node_ref: ConnectionNodeRef,
    pub connected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencySample {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    pub measured_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub real_delay_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress: Option<EgressSample>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub verdicts: HashMap<String, VerdictRecord>,
}

/// The public egress address and location a node was observed to exit from,
/// measured through the node's own tunnel during a probe session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressSample {
    pub address: String,
    pub location: String,
    pub checked_at: DateTime<Utc>,
}

/// The most recent availability verdict for one node and one service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictRecord {
    pub result: AvailabilityVerdict,
    pub checked_at: DateTime<Utc>,
}
