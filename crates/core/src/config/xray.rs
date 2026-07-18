use serde_json::Value;

use crate::config::v2ray::{V2rayFamilyBackend, generate_v2ray_family_config};
use crate::config::{ConfigError, ConfigGenerator};
use crate::models::{AppSettings, ProxyNode, RoutingRule, VlessConfig};

pub struct XrayGenerator;

/// fwmark stamped on xray's own outbound sockets in TUN mode. The privileged
/// route helper installs a matching policy rule that diverts marked packets past
/// the TUN table, so `direct` traffic egresses the real interface instead of
/// looping back into the tunnel. Must match `XRAY_FWMARK` in the netctl crate.
pub const XRAY_TUN_FWMARK: u32 = 255;

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

        if settings.tun.enabled {
            apply_tun_fwmark(&mut config);
        }

        Ok(config)
    }
}

/// Stamps every dialing outbound with the TUN fwmark via `streamSettings.sockopt.mark`
/// so the route helper's policy rules exempt xray's own traffic from the tunnel.
/// The blackhole `block` and internal-resolver `dns` outbounds never dial, so
/// they are skipped.
fn apply_tun_fwmark(config: &mut Value) {
    let Some(outbounds) = config["outbounds"].as_array_mut() else {
        return;
    };
    for outbound in outbounds {
        if outbound["protocol"] == "blackhole" || outbound["protocol"] == "dns" {
            continue;
        }
        outbound["streamSettings"]["sockopt"]["mark"] = Value::from(XRAY_TUN_FWMARK);
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

/// Builds a single xray outbound (v2ray-family outbound plus xray-specific
/// XTLS flow extensions) for the given node and tag. Shared with the Real Delay
/// probe config generator.
pub(crate) fn build_xray_outbound(node: &ProxyNode, tag: &str) -> Value {
    let mut outbound = crate::config::v2ray::build_family_outbound(node, tag);
    if let ProxyNode::Vless(c) = node {
        apply_xray_vless_extensions(&mut outbound, c);
    }
    outbound
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

    #[test]
    fn test_xray_generator_emits_tun_inbound_when_enabled() {
        let mut settings = AppSettings::default();
        settings.tun.enabled = true;

        let config = XrayGenerator
            .generate(&[xray_vless_with_xtls()], &[], &settings)
            .unwrap();

        let tun = config["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["protocol"] == "tun")
            .expect("xray tun inbound missing");
        assert_eq!(tun["settings"]["autoOutboundsInterface"], "auto");
        assert_eq!(tun["sniffing"]["enabled"], true);
    }

    #[test]
    fn test_xray_tun_marks_dialing_outbounds() {
        let mut settings = AppSettings::default();
        settings.tun.enabled = true;

        let config = XrayGenerator
            .generate(&[xray_vless_with_xtls()], &[], &settings)
            .unwrap();

        for outbound in config["outbounds"].as_array().unwrap() {
            if outbound["protocol"] == "blackhole" || outbound["protocol"] == "dns" {
                assert!(outbound["streamSettings"]["sockopt"]["mark"].is_null());
            } else {
                assert_eq!(outbound["streamSettings"]["sockopt"]["mark"], 255);
            }
        }
    }

    #[test]
    fn test_xray_no_fwmark_when_tun_disabled() {
        let config = XrayGenerator
            .generate(&[xray_vless_with_xtls()], &[], &AppSettings::default())
            .unwrap();

        for outbound in config["outbounds"].as_array().unwrap() {
            assert!(outbound["streamSettings"]["sockopt"]["mark"].is_null());
        }
    }
}
