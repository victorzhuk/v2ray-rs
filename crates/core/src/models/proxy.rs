use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "lowercase")]
pub enum ProxyNode {
    Vless(VlessConfig),
    Vmess(VmessConfig),
    Shadowsocks(ShadowsocksConfig),
    Trojan(TrojanConfig),
}

impl ProxyNode {
    pub fn remark(&self) -> Option<&str> {
        match self {
            Self::Vless(c) => c.remark.as_deref(),
            Self::Vmess(c) => c.remark.as_deref(),
            Self::Shadowsocks(c) => c.remark.as_deref(),
            Self::Trojan(c) => c.remark.as_deref(),
        }
    }

    pub fn address(&self) -> &str {
        match self {
            Self::Vless(c) => &c.address,
            Self::Vmess(c) => &c.address,
            Self::Shadowsocks(c) => &c.address,
            Self::Trojan(c) => &c.address,
        }
    }

    pub fn port(&self) -> u16 {
        match self {
            Self::Vless(c) => c.port,
            Self::Vmess(c) => c.port,
            Self::Shadowsocks(c) => c.port,
            Self::Trojan(c) => c.port,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowsocksConfig {
    pub address: String,
    pub port: u16,
    pub method: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TransportSettings {
    Tcp,
    Ws(WsSettings),
    Grpc(GrpcSettings),
    H2(H2Settings),
}

impl Default for TransportSettings {
    fn default() -> Self {
        Self::Tcp
    }
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
                verify: true,
                fingerprint: None,
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
                alpn: vec![],
                verify: true,
                fingerprint: None,
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
}
