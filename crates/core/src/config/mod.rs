mod common;
pub mod dns_plan;
pub mod effective_dns;
mod probe;
mod redact;
pub(crate) mod singbox;
#[cfg(test)]
mod test_fixtures;
pub(crate) mod v2ray;
mod writer;
pub(crate) mod xray;

pub use dns_plan::{ResolvedDnsPlan, ResolvedDnsServer};
pub use redact::redact_json;

pub use probe::{
    EGRESS_TAG_PREFIX, PROBE_TAG_PREFIX, ProbeConfigGenerator, SingboxProbeGenerator,
    V2rayProbeGenerator, XrayProbeGenerator, egress_inbound_ports, egress_tag,
    generate_clash_api_secret, pick_free_loopback_ports, probe_generator_for, probe_tag,
};
pub use singbox::SingboxGenerator;
pub use v2ray::V2rayGenerator;
pub use writer::ConfigWriter;
pub use xray::XRAY_TUN_FWMARK;
pub use xray::XrayGenerator;

use crate::models::{
    AppSettings, BackendType, DnsProtocol, DnsValidationError, ProxyNode, ProxyNodeValidationError,
    RoutingRule, ValidationError,
};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("no enabled proxy nodes")]
    NoNodes,
    #[error(transparent)]
    InvalidProxyNode(#[from] ProxyNodeValidationError),
    #[error(transparent)]
    InvalidDns(#[from] DnsValidationError),
    #[error("invalid tun config: {0}")]
    InvalidTun(#[from] ValidationError),
    #[error("invalid settings: {0}")]
    InvalidSettings(ValidationError),
    #[error("invalid routing rule: {0}")]
    InvalidRule(ValidationError),
    #[error("dns protocol {protocol:?} is not supported by backend {backend} for server '{tag}'")]
    UnsupportedDnsProtocol {
        backend: BackendType,
        protocol: DnsProtocol,
        tag: String,
    },
    #[error("transport not supported by backend {backend} for node '{node}'")]
    UnsupportedTransport { backend: BackendType, node: String },
    #[error("security {feature} not supported by backend {backend} for node '{node}'; use xray")]
    UnsupportedSecurity {
        backend: BackendType,
        node: String,
        feature: &'static str,
    },
    #[error(
        "node '{node}' disables certificate verification, which backend {backend} does not support; enable verification or use sing-box"
    )]
    UnsupportedTlsVerification { backend: BackendType, node: String },
    #[error("serialize config: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("write config: {0}")]
    Io(#[from] std::io::Error),
}

pub trait ConfigGenerator {
    fn generate(
        &self,
        nodes: &ResolvedNodeSet<'_>,
        rules: &[RoutingRule],
        settings: &AppSettings,
    ) -> Result<serde_json::Value, ConfigError>;
}

/// The typed node set a config generator accepts: one connected node plus every
/// additional node a routing rule can select as its detour.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedNodeSet<'a> {
    pub primary: &'a ProxyNode,
    pub detour_nodes: &'a [ProxyNode],
}

impl<'a> ResolvedNodeSet<'a> {
    pub fn new(primary: &'a ProxyNode, detour_nodes: &'a [ProxyNode]) -> Self {
        Self {
            primary,
            detour_nodes,
        }
    }

    /// Splits a positional node slice in the established order: the connected
    /// node first, every detour node after it.
    pub fn from_nodes(nodes: &'a [ProxyNode]) -> Result<Self, ConfigError> {
        let (primary, detour_nodes) = nodes.split_first().ok_or(ConfigError::NoNodes)?;
        Ok(Self {
            primary,
            detour_nodes,
        })
    }

    /// Materializes the positional slice the generator internals still build
    /// their outbound tags from.
    pub fn to_vec(&self) -> Vec<ProxyNode> {
        let mut nodes = Vec::with_capacity(1 + self.detour_nodes.len());
        nodes.push(self.primary.clone());
        nodes.extend_from_slice(self.detour_nodes);
        nodes
    }
}

pub fn generator_for(backend: BackendType) -> Box<dyn ConfigGenerator> {
    match backend {
        BackendType::V2ray => Box::new(V2rayGenerator),
        BackendType::Xray => Box::new(XrayGenerator),
        BackendType::SingBox => Box::new(SingboxGenerator),
    }
}
