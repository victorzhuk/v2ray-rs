use serde_json::Value;

use crate::config::v2ray::{V2rayFamilyBackend, generate_v2ray_family_config};
use crate::config::{ConfigError, ConfigGenerator};
use crate::models::{AppSettings, ProxyNode, RoutingRule, VlessConfig};

pub struct XrayGenerator;

impl ConfigGenerator for XrayGenerator {
    fn generate(
        &self,
        nodes: &[ProxyNode],
        rules: &[RoutingRule],
        settings: &AppSettings,
    ) -> Result<Value, ConfigError> {
        if nodes.is_empty() {
            return Err(ConfigError::NoNodes);
        }

        let mut config =
            generate_v2ray_family_config(nodes, rules, settings, V2rayFamilyBackend::Xray);

        patch_xray_outbounds(&mut config, nodes);
        Ok(config)
    }
}

fn patch_xray_outbounds(config: &mut Value, nodes: &[ProxyNode]) {
    let Some(outbounds) = config["outbounds"].as_array_mut() else {
        return;
    };

    for (i, node) in nodes.iter().enumerate() {
        if let ProxyNode::Vless(c) = node
            && let Some(outbound) = outbounds.get_mut(i)
        {
            apply_xray_vless_extensions(outbound, c);
        }
    }
}

fn apply_xray_vless_extensions(outbound: &mut Value, c: &VlessConfig) {
    if let Some(ref flow) = c.flow
        && is_xtls_flow(flow)
        && let Some(users) = outbound["settings"]["vnext"][0]["users"].as_array_mut()
        && let Some(user) = users.first_mut()
    {
        user["flow"] = serde_json::json!(flow);
    }
}

fn is_xtls_flow(flow: &str) -> bool {
    flow.starts_with("xtls-rprx-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;

    fn xray_vless_with_xtls() -> ProxyNode {
        ProxyNode::Vless(VlessConfig {
            address: "xray.example.com".into(),
            port: 443,
            uuid: "test-uuid-xtls".into(),
            encryption: Some("none".into()),
            flow: Some("xtls-rprx-vision".into()),
            transport: TransportSettings::Tcp,
            tls: Some(TlsSettings {
                server_name: Some("xray.example.com".into()),
                fingerprint: Some("chrome".into()),
                ..Default::default()
            }),
            remark: Some("XTLS Node".into()),
        })
    }

    fn vless_without_xtls() -> ProxyNode {
        ProxyNode::Vless(VlessConfig {
            address: "plain.example.com".into(),
            port: 443,
            uuid: "test-uuid-plain".into(),
            encryption: Some("none".into()),
            flow: None,
            transport: TransportSettings::Ws(WsSettings {
                path: "/ws".into(),
                host: None,
                headers: Default::default(),
            }),
            tls: Some(TlsSettings {
                server_name: Some("plain.example.com".into()),
                ..Default::default()
            }),
            remark: Some("Plain VLESS".into()),
        })
    }

    #[test]
    fn test_xray_xtls_flow_applied() {
        let generator = XrayGenerator;
        let config = generator
            .generate(&[xray_vless_with_xtls()], &[], &AppSettings::default())
            .unwrap();

        let outbound = &config["outbounds"][0];
        let user = &outbound["settings"]["vnext"][0]["users"][0];
        assert_eq!(user["flow"], "xtls-rprx-vision");
        assert_eq!(outbound["streamSettings"]["security"], "tls");
    }

    #[test]
    fn test_xray_non_xtls_unmodified() {
        let generator = XrayGenerator;
        let config = generator
            .generate(&[vless_without_xtls()], &[], &AppSettings::default())
            .unwrap();

        let outbound = &config["outbounds"][0];
        assert_eq!(outbound["streamSettings"]["security"], "tls");
    }

    #[test]
    fn test_xray_dns_accepts_dot_and_doq_and_falls_back_from_h3() {
        let generator = XrayGenerator;
        let mut settings = AppSettings::default();
        settings.dns.enabled = true;
        settings.dns.servers = vec![
            DnsServerConfig {
                tag: "dot".into(),
                protocol: DnsProtocol::Dot,
                address: "dns.google".into(),
                port: None,
                detour: None,
            },
            DnsServerConfig {
                tag: "doq".into(),
                protocol: DnsProtocol::Doq,
                address: "dns.adguard.com".into(),
                port: None,
                detour: None,
            },
            DnsServerConfig {
                tag: "h3".into(),
                protocol: DnsProtocol::H3,
                address: "cloudflare-dns.com".into(),
                port: None,
                detour: None,
            },
        ];

        let config = generator
            .generate(&[vless_without_xtls()], &[], &settings)
            .unwrap();

        let servers = config["dns"]["servers"].as_array().unwrap();
        assert_eq!(servers[0].as_str(), Some("tls://dns.google"));
        assert_eq!(servers[1].as_str(), Some("quic://dns.adguard.com"));
        assert_eq!(
            servers[2].as_str(),
            Some("https://cloudflare-dns.com/dns-query")
        );
    }

    #[test]
    fn test_xray_mixed_nodes() {
        let generator = XrayGenerator;
        let nodes = vec![
            xray_vless_with_xtls(),
            vless_without_xtls(),
            ProxyNode::Shadowsocks(ShadowsocksConfig {
                address: "ss.example.com".into(),
                port: 8388,
                method: "aes-256-gcm".into(),
                password: "secret".into(),
                remark: Some("SS".into()),
            }),
        ];

        let config = generator
            .generate(&nodes, &[], &AppSettings::default())
            .unwrap();

        let outbounds = config["outbounds"].as_array().unwrap();
        assert_eq!(outbounds.len(), 5);

        assert_eq!(outbounds[0]["streamSettings"]["security"], "tls");
        assert_eq!(outbounds[1]["streamSettings"]["security"], "tls");
        assert_eq!(outbounds[2]["protocol"], "shadowsocks");
    }

    #[test]
    fn test_xray_error_on_empty_nodes() {
        let generator = XrayGenerator;
        let result = generator.generate(&[], &[], &AppSettings::default());
        assert!(result.is_err());
    }
}
