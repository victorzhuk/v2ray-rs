mod common;
mod singbox;
#[cfg(test)]
mod test_fixtures;
pub(crate) mod v2ray;
mod writer;
mod xray;

pub use singbox::SingboxGenerator;
pub use v2ray::V2rayGenerator;
pub use writer::ConfigWriter;
pub use xray::XrayGenerator;

use std::path::Path;

use crate::models::{AppSettings, BackendType, ProxyNode, RoutingRule};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("no enabled proxy nodes")]
    NoNodes,
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
        geodata_dir: Option<&Path>,
    ) -> Result<serde_json::Value, ConfigError>;
}

pub fn generator_for(backend: BackendType) -> Box<dyn ConfigGenerator> {
    match backend {
        BackendType::V2ray => Box::new(V2rayGenerator),
        BackendType::Xray => Box::new(XrayGenerator),
        BackendType::SingBox => Box::new(SingboxGenerator),
    }
}
