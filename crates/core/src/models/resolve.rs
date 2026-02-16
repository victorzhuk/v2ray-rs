use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::BackendType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoResolveStrategy {
    ListOrder,
    LowestLatency,
    Random,
    LastSuccessful,
    GeoAware,
}

impl Default for AutoResolveStrategy {
    fn default() -> Self {
        Self::ListOrder
    }
}

impl fmt::Display for AutoResolveStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ListOrder => f.write_str("list order"),
            Self::LowestLatency => f.write_str("lowest latency"),
            Self::Random => f.write_str("random"),
            Self::LastSuccessful => f.write_str("last successful"),
            Self::GeoAware => f.write_str("geo-aware"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionMetadata {
    pub subscription_id: Uuid,
    pub subscription_name: String,
    pub node_index: usize,
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
    pub subscription_id: Uuid,
    pub node_index: usize,
    pub connected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencySample {
    pub latency_ms: u64,
    pub measured_at: DateTime<Utc>,
}
