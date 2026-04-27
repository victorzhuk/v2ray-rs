use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "lowercase")]
pub enum ProxyNode {
    Vless(VlessConfig),
    Vmess(VmessConfig),
    Shadowsocks(ShadowsocksConfig),
    Trojan(TrojanConfig),
}

impl ProxyNode {
    #[must_use]
    pub fn remark(&self) -> Option<&str> {
        match self {
            Self::Vless(c) => c.remark.as_deref(),
            Self::Vmess(c) => c.remark.as_deref(),
            Self::Shadowsocks(c) => c.remark.as_deref(),
            Self::Trojan(c) => c.remark.as_deref(),
        }
    }

    #[must_use]
    pub fn address(&self) -> &str {
        match self {
            Self::Vless(c) => &c.address,
            Self::Vmess(c) => &c.address,
            Self::Shadowsocks(c) => &c.address,
            Self::Trojan(c) => &c.address,
        }
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        match self {
            Self::Vless(c) => c.port,
            Self::Vmess(c) => c.port,
            Self::Shadowsocks(c) => c.port,
            Self::Trojan(c) => c.port,
        }
    }

    #[must_use]
    pub fn protocol_name(&self) -> &'static str {
        match self {
            Self::Vless(_) => "vless",
            Self::Vmess(_) => "vmess",
            Self::Shadowsocks(_) => "shadowsocks",
            Self::Trojan(_) => "trojan",
        }
    }

    pub fn validate(&self) -> Result<(), ProxyNodeValidationError> {
        validate_common(self.protocol_name(), self.address(), self.port())?;

        match self {
            Self::Vless(c) => {
                require_non_empty("vless", "uuid", &c.uuid)?;
            }
            Self::Vmess(c) => {
                require_non_empty("vmess", "uuid", &c.uuid)?;
                require_non_empty("vmess", "security", &c.security)?;
            }
            Self::Shadowsocks(c) => {
                require_non_empty("shadowsocks", "method", &c.method)?;
                require_non_empty("shadowsocks", "password", &c.password)?;
            }
            Self::Trojan(c) => {
                require_non_empty("trojan", "password", &c.password)?;
            }
        }

        if let Some(tls) = self.tls() {
            validate_tls(self.protocol_name(), tls)?;
        }

        Ok(())
    }

    fn tls(&self) -> Option<&TlsSettings> {
        match self {
            Self::Vless(c) => c.tls.as_ref(),
            Self::Vmess(c) => c.tls.as_ref(),
            Self::Shadowsocks(_) => None,
            Self::Trojan(c) => c.tls.as_ref(),
        }
    }
}

fn validate_common(
    protocol: &'static str,
    address: &str,
    port: u16,
) -> Result<(), ProxyNodeValidationError> {
    require_non_empty(protocol, "address", address)?;
    if port == 0 {
        return Err(ProxyNodeValidationError::InvalidPort {
            protocol,
            field: "port",
        });
    }
    Ok(())
}

fn validate_tls(protocol: &'static str, tls: &TlsSettings) -> Result<(), ProxyNodeValidationError> {
    if tls.reality {
        require_non_empty(
            protocol,
            "tls.public_key",
            tls.public_key.as_deref().unwrap_or(""),
        )?;
    }
    Ok(())
}

fn require_non_empty(
    protocol: &'static str,
    field: &'static str,
    value: &str,
) -> Result<(), ProxyNodeValidationError> {
    if value.trim().is_empty() {
        return Err(ProxyNodeValidationError::MissingField { protocol, field });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProxyNodeValidationError {
    #[error("{protocol} node requires non-empty field '{field}'")]
    MissingField {
        protocol: &'static str,
        field: &'static str,
    },
    #[error("{protocol} node requires a non-zero '{field}'")]
    InvalidPort {
        protocol: &'static str,
        field: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VlessConfig {
    pub address: String,
    pub port: u16,
    pub uuid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    #[serde(default)]
    pub transport: TransportSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VmessConfig {
    pub address: String,
    pub port: u16,
    pub uuid: String,
    #[serde(default)]
    pub alter_id: u32,
    #[serde(default = "default_vmess_security")]
    pub security: String,
    #[serde(default)]
    pub transport: TransportSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
}

fn default_vmess_security() -> String {
    "auto".to_string()
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowsocksConfig {
    pub address: String,
    pub port: u16,
    pub method: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
}

impl std::fmt::Debug for ShadowsocksConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShadowsocksConfig")
            .field("address", &self.address)
            .field("port", &self.port)
            .field("method", &self.method)
            .field("password", &"***")
            .field("remark", &self.remark)
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct TrojanConfig {
    pub address: String,
    pub port: u16,
    pub password: String,
    #[serde(default)]
    pub transport: TransportSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
}

impl std::fmt::Debug for TrojanConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrojanConfig")
            .field("address", &self.address)
            .field("port", &self.port)
            .field("password", &"***")
            .field("transport", &self.transport)
            .field("tls", &self.tls)
            .field("remark", &self.remark)
            .finish()
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TransportSettings {
    #[default]
    Tcp,
    Ws(WsSettings),
    Grpc(GrpcSettings),
    H2(H2Settings),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WsSettings {
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrpcSettings {
    pub service_name: String,
    #[serde(default)]
    pub multi_mode: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct H2Settings {
    #[serde(default)]
    pub host: Vec<String>,
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TlsSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(default)]
    pub alpn: Vec<String>,
    #[serde(default = "default_true")]
    pub verify: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub reality: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spider_x: Option<String>,
}

impl Default for TlsSettings {
    fn default() -> Self {
        Self {
            server_name: None,
            alpn: Vec::new(),
            verify: true,
            fingerprint: None,
            reality: false,
            public_key: None,
            short_id: None,
            spider_x: None,
        }
    }
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_vless() -> ProxyNode {
        ProxyNode::Vless(VlessConfig {
            address: "example.com".into(),
            port: 443,
            uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
            encryption: Some("none".into()),
            flow: Some("xtls-rprx-vision".into()),
            transport: TransportSettings::Ws(WsSettings {
                path: "/ws".into(),
                host: Some("example.com".into()),
                headers: Default::default(),
            }),
            tls: Some(TlsSettings {
                server_name: Some("example.com".into()),
                alpn: vec!["h2".into()],
                ..Default::default()
            }),
            remark: Some("Test VLESS".into()),
        })
    }

    fn sample_vmess() -> ProxyNode {
        ProxyNode::Vmess(VmessConfig {
            address: "vmess.example.com".into(),
            port: 8443,
            uuid: "123e4567-e89b-12d3-a456-426614174000".into(),
            alter_id: 0,
            security: "auto".into(),
            transport: TransportSettings::Tcp,
            tls: None,
            remark: Some("Test VMess".into()),
        })
    }

    fn sample_ss() -> ProxyNode {
        ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: "ss.example.com".into(),
            port: 8388,
            method: "aes-256-gcm".into(),
            password: "secret".into(),
            remark: Some("Test SS".into()),
        })
    }

    fn sample_trojan() -> ProxyNode {
        ProxyNode::Trojan(TrojanConfig {
            address: "trojan.example.com".into(),
            port: 443,
            password: "trojan-pass".into(),
            transport: TransportSettings::Tcp,
            tls: Some(TlsSettings {
                server_name: Some("trojan.example.com".into()),
                ..Default::default()
            }),
            remark: Some("Test Trojan".into()),
        })
    }

    #[test]
    fn test_vless_serialization_roundtrip() {
        let node = sample_vless();
        let json = serde_json::to_string(&node).unwrap();
        let deserialized: ProxyNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, deserialized);
    }

    #[test]
    fn test_vmess_serialization_roundtrip() {
        let node = sample_vmess();
        let json = serde_json::to_string(&node).unwrap();
        let deserialized: ProxyNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, deserialized);
    }

    #[test]
    fn test_shadowsocks_serialization_roundtrip() {
        let node = sample_ss();
        let json = serde_json::to_string(&node).unwrap();
        let deserialized: ProxyNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, deserialized);
    }

    #[test]
    fn test_trojan_serialization_roundtrip() {
        let node = sample_trojan();
        let json = serde_json::to_string(&node).unwrap();
        let deserialized: ProxyNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, deserialized);
    }

    #[test]
    fn test_proxy_node_accessors() {
        let node = sample_vless();
        assert_eq!(node.remark(), Some("Test VLESS"));
        assert_eq!(node.address(), "example.com");
        assert_eq!(node.port(), 443);
    }

    #[test]
    fn test_tagged_serialization() {
        let node = sample_ss();
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains(r#""protocol":"shadowsocks""#));
    }

    #[test]
    fn test_default_transport() {
        assert_eq!(TransportSettings::default(), TransportSettings::Tcp);
    }

    #[test]
    fn test_validate_rejects_empty_required_fields() {
        let node = ProxyNode::Trojan(TrojanConfig {
            address: "example.com".into(),
            port: 443,
            password: String::new(),
            transport: TransportSettings::Tcp,
            tls: None,
            remark: None,
        });

        assert_eq!(
            node.validate(),
            Err(ProxyNodeValidationError::MissingField {
                protocol: "trojan",
                field: "password",
            })
        );
    }

    #[test]
    fn test_validate_rejects_empty_address() {
        let node = ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: "   ".into(),
            port: 8388,
            method: "aes-256-gcm".into(),
            password: "secret".into(),
            remark: None,
        });

        assert_eq!(
            node.validate(),
            Err(ProxyNodeValidationError::MissingField {
                protocol: "shadowsocks",
                field: "address",
            })
        );
    }

    #[test]
    fn test_validate_rejects_zero_port() {
        let node = ProxyNode::Vmess(VmessConfig {
            address: "example.com".into(),
            port: 0,
            uuid: "123e4567-e89b-12d3-a456-426614174000".into(),
            alter_id: 0,
            security: "auto".into(),
            transport: TransportSettings::Tcp,
            tls: None,
            remark: None,
        });

        assert_eq!(
            node.validate(),
            Err(ProxyNodeValidationError::InvalidPort {
                protocol: "vmess",
                field: "port",
            })
        );
    }

    #[test]
    fn test_validate_rejects_reality_without_public_key() {
        let node = ProxyNode::Vless(VlessConfig {
            address: "example.com".into(),
            port: 443,
            uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
            encryption: Some("none".into()),
            flow: None,
            transport: TransportSettings::Tcp,
            tls: Some(TlsSettings {
                reality: true,
                ..Default::default()
            }),
            remark: None,
        });

        assert_eq!(
            node.validate(),
            Err(ProxyNodeValidationError::MissingField {
                protocol: "vless",
                field: "tls.public_key",
            })
        );
    }
}
