mod common;
mod probe;
mod redact;
pub(crate) mod singbox;
#[cfg(test)]
mod test_fixtures;
pub(crate) mod v2ray;
mod writer;
pub(crate) mod xray;

pub use redact::redact_json;

pub use probe::{
    PROBE_TAG_PREFIX, ProbeConfigGenerator, SingboxProbeGenerator, V2rayProbeGenerator,
    XrayProbeGenerator, probe_generator_for, probe_tag,
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
    #[error("dns protocol {protocol:?} is not supported by backend {backend} for server '{tag}'")]
    UnsupportedDnsProtocol {
        backend: BackendType,
        protocol: DnsProtocol,
        tag: String,
    },
    #[error("serialize config: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("write config: {0}")]
    Io(#[from] std::io::Error),
}

pub trait ConfigGenerator {
    fn generate(
        &self,
        nodes: &[ProxyNode],
        rules: &[RoutingRule],
        settings: &AppSettings,
    ) -> Result<serde_json::Value, ConfigError>;
}

pub fn generator_for(backend: BackendType) -> Box<dyn ConfigGenerator> {
    match backend {
        BackendType::V2ray => Box::new(V2rayGenerator),
        BackendType::Xray => Box::new(XrayGenerator),
        BackendType::SingBox => Box::new(SingboxGenerator),
    }
}
