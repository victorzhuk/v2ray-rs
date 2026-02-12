use std::path::Path;

use serde_json::Value;

use crate::config::v2ray::V2rayGenerator;
use crate::config::{ConfigError, ConfigGenerator};
use crate::models::{AppSettings, ProxyNode, RoutingRule, TransportSettings, VlessConfig};

pub struct XrayGenerator;

impl ConfigGenerator for XrayGenerator {
    fn generate(
        &self,
        nodes: &[ProxyNode],
        rules: &[RoutingRule],
        settings: &AppSettings,
        _geodata_dir: Option<&Path>,
    ) -> Result<Value, ConfigError> {
        let v2ray = V2rayGenerator;
        let mut config = v2ray.generate(nodes, rules, settings, None)?;

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
    {
        if let Some(users) = outbound["settings"]["vnext"][0]["users"].as_array_mut()
            && let Some(user) = users.first_mut()
        {
            user["flow"] = serde_json::json!(flow);
        }

        if matches!(c.transport, TransportSettings::Tcp) && c.tls.is_some() {
            outbound["streamSettings"]["security"] = serde_json::json!("xtls");
        }
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
                alpn: vec![],
                verify: true,
                fingerprint: Some("chrome".into()),
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
                alpn: vec![],
                verify: true,
                fingerprint: None,
            }),
            remark: Some("Plain VLESS".into()),
        })
    }

    #[test]
    fn test_xray_xtls_flow_applied() {
        let generator = XrayGenerator;
        let config = generator
            .generate(
                &[xray_vless_with_xtls()],
                &[],
                &AppSettings::default(),
                None,
            )
            .unwrap();

        let outbound = &config["outbounds"][0];
        let user = &outbound["settings"]["vnext"][0]["users"][0];
        assert_eq!(user["flow"], "xtls-rprx-vision");
        assert_eq!(outbound["streamSettings"]["security"], "xtls");
    }

    #[test]
    fn test_xray_non_xtls_unmodified() {
        let generator = XrayGenerator;
        let config = generator
            .generate(&[vless_without_xtls()], &[], &AppSettings::default(), None)
            .unwrap();

        let outbound = &config["outbounds"][0];
        assert_eq!(outbound["streamSettings"]["security"], "tls");
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
            .generate(&nodes, &[], &AppSettings::default(), None)
            .unwrap();

        let outbounds = config["outbounds"].as_array().unwrap();
        // 3 proxy + direct + block = 5
        assert_eq!(outbounds.len(), 5);

        assert_eq!(outbounds[0]["streamSettings"]["security"], "xtls");
        assert_eq!(outbounds[1]["streamSettings"]["security"], "tls");
        assert_eq!(outbounds[2]["protocol"], "shadowsocks");
    }

    #[test]
    fn test_xray_error_on_empty_nodes() {
        let generator = XrayGenerator;
        let result = generator.generate(&[], &[], &AppSettings::default(), None);
        assert!(result.is_err());
    }
}
