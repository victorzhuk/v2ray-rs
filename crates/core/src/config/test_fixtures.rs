#[cfg(test)]
pub(crate) mod fixtures {
    use crate::models::*;

    pub fn default_settings() -> AppSettings {
        AppSettings::default()
    }

    pub fn vless_node() -> ProxyNode {
        ProxyNode::Vless(VlessConfig {
            address: "example.com".into(),
            port: 443,
            uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
            encryption: Some("none".into()),
            flow: None,
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

    pub fn vmess_node() -> ProxyNode {
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

    pub fn ss_node() -> ProxyNode {
        ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: "ss.example.com".into(),
            port: 8388,
            method: "aes-256-gcm".into(),
            password: "secret".into(),
            remark: Some("Test SS".into()),
        })
    }

    pub fn trojan_node() -> ProxyNode {
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
}
