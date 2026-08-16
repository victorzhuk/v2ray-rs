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

        patch_xray_outbounds(&mut config, nodes, settings);

        if settings.tun.enabled {
            apply_tun_fwmark(&mut config);
            apply_tun_domain_strategy(&mut config, tun_domain_strategy(settings));
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

/// Routes the dialer's own hostname lookups through xray's built-in resolver
/// instead of the OS one.
///
/// The default `AsIs` hands a proxy server's hostname to Go's resolver, which
/// reads `/etc/resolv.conf` and queries on an unmarked socket — a socket the TUN
/// captures, so the lookup needed to build the tunnel is sent through it. Paired
/// with pinned `dns.hosts` entries the built-in resolver answers locally and the
/// loop never forms; xray's own docs recommend exactly this pinning for
/// transparent-proxy setups.
fn apply_tun_domain_strategy(config: &mut Value, strategy: &str) {
    let Some(outbounds) = config["outbounds"].as_array_mut() else {
        return;
    };
    for outbound in outbounds {
        if outbound["protocol"] == "blackhole" || outbound["protocol"] == "dns" {
            continue;
        }
        outbound["streamSettings"]["sockopt"]["domainStrategy"] = Value::from(strategy);
    }
}

fn patch_xray_outbounds(config: &mut Value, nodes: &[ProxyNode], settings: &AppSettings) {
    let Some(outbounds) = config["outbounds"].as_array_mut() else {
        return;
    };

    for (i, node) in nodes.iter().enumerate() {
        let Some(outbound) = outbounds.get_mut(i) else {
            continue;
        };
        if let ProxyNode::Vless(c) = node {
            apply_xray_vless_extensions(outbound, c);
        }
        migrate_ws_host(outbound);
        apply_ws_heartbeat(outbound, settings.ws_heartbeat_secs);
    }
}

fn apply_ws_heartbeat(outbound: &mut Value, secs: u32) {
    if secs == 0 {
        return;
    }
    let Some(ws) = outbound
        .get_mut("streamSettings")
        .and_then(|s| s.get_mut("wsSettings"))
        .and_then(|w| w.as_object_mut())
    else {
        return;
    };
    ws.insert("heartbeatPeriod".to_string(), Value::from(secs));
}

/// Builds a single xray outbound (v2ray-family outbound plus xray-specific
/// XTLS flow extensions) for the given node and tag. Shared with the Real Delay
/// probe config generator.
pub(crate) fn build_xray_outbound(node: &ProxyNode, tag: &str) -> Value {
    let mut outbound = crate::config::v2ray::build_family_outbound(node, tag);
    if let ProxyNode::Vless(c) = node {
        apply_xray_vless_extensions(&mut outbound, c);
    }
    migrate_ws_host(&mut outbound);
    outbound
}

/// xray 26 warns on every start that `wsSettings.headers.Host` is deprecated and
/// due for removal in favour of a dedicated `host` field, which also outranks the
/// header when both are present. Move it across so the config outlives the removal.
fn migrate_ws_host(outbound: &mut Value) {
    let Some(ws) = outbound
        .get_mut("streamSettings")
        .and_then(|s| s.get_mut("wsSettings"))
        .and_then(|w| w.as_object_mut())
    else {
        return;
    };
    let Some(headers) = ws.get_mut("headers").and_then(|h| h.as_object_mut()) else {
        return;
    };
    let Some(key) = headers
        .keys()
        .find(|k| k.eq_ignore_ascii_case("host"))
        .cloned()
    else {
        return;
    };
    let Some(host) = headers.remove(&key) else {
        return;
    };
    if headers.is_empty() {
        ws.remove("headers");
    }
    ws.insert("host".to_string(), host);
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

/// `Use*` rather than `Force*`: a node whose address family the resolver cannot
/// satisfy should still be dialled as-is instead of failing the connection.
fn tun_domain_strategy(settings: &AppSettings) -> &'static str {
    match settings.dns.strategy {
        crate::models::DnsStrategy::Ipv6Only | crate::models::DnsStrategy::PreferIpv6 => "UseIPv6",
        _ => "UseIPv4",
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

    fn ws_vless_with_host_header(headers: &[(&str, &str)]) -> ProxyNode {
        ProxyNode::Vless(VlessConfig {
            address: "ws.example.com".into(),
            port: 443,
            uuid: "test-uuid-ws".into(),
            encryption: Some("none".into()),
            flow: None,
            transport: TransportSettings::Ws(WsSettings {
                path: "/ws".into(),
                host: Some("cdn.example.com".into()),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            }),
            tls: Some(TlsSettings {
                server_name: Some("ws.example.com".into()),
                ..Default::default()
            }),
            remark: Some("WS Node".into()),
        })
    }

    #[test]
    fn test_ws_host_header_moves_to_dedicated_field() {
        let node = ws_vless_with_host_header(&[]);
        let config = XrayGenerator
            .generate(&[node], &[], &AppSettings::default())
            .unwrap();

        let ws = &config["outbounds"][0]["streamSettings"]["wsSettings"];
        assert_eq!(ws["host"], "cdn.example.com");
        assert!(
            ws.get("headers").is_none(),
            "Host was the only header, so headers should be gone: {ws}"
        );
    }

    #[test]
    fn test_ws_host_migration_keeps_other_headers() {
        let node = ws_vless_with_host_header(&[("Host", "cdn.example.com"), ("X-Tag", "keep")]);
        let config = XrayGenerator
            .generate(&[node], &[], &AppSettings::default())
            .unwrap();

        let ws = &config["outbounds"][0]["streamSettings"]["wsSettings"];
        assert_eq!(ws["host"], "cdn.example.com");
        assert_eq!(ws["headers"]["X-Tag"], "keep");
        assert!(ws["headers"].get("Host").is_none());
    }

    #[test]
    fn test_tun_sets_domain_strategy_so_the_dialer_uses_the_builtin_resolver() {
        let settings = AppSettings {
            tun: TunConfig {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let config = XrayGenerator
            .generate(&[ws_vless_with_host_header(&[])], &[], &settings)
            .unwrap();

        for outbound in config["outbounds"].as_array().unwrap() {
            let sockopt = &outbound["streamSettings"]["sockopt"];
            if outbound["protocol"] == "blackhole" || outbound["protocol"] == "dns" {
                assert!(sockopt.get("domainStrategy").is_none());
            } else {
                assert_eq!(
                    sockopt["domainStrategy"], "UseIPv4",
                    "dialer must not fall back to the OS resolver under TUN: {outbound}"
                );
            }
        }
    }

    #[test]
    fn test_no_domain_strategy_without_tun() {
        let config = XrayGenerator
            .generate(
                &[ws_vless_with_host_header(&[])],
                &[],
                &AppSettings::default(),
            )
            .unwrap();

        assert!(
            config["outbounds"][0]["streamSettings"]["sockopt"]
                .get("domainStrategy")
                .is_none()
        );
    }

    #[test]
    fn test_ws_heartbeat_emitted_when_configured() {
        let settings = AppSettings {
            ws_heartbeat_secs: 30,
            ..Default::default()
        };

        let config = XrayGenerator
            .generate(&[ws_vless_with_host_header(&[])], &[], &settings)
            .unwrap();

        assert_eq!(
            config["outbounds"][0]["streamSettings"]["wsSettings"]["heartbeatPeriod"],
            30
        );
    }

    #[test]
    fn test_ws_heartbeat_absent_by_default() {
        let config = XrayGenerator
            .generate(
                &[ws_vless_with_host_header(&[])],
                &[],
                &AppSettings::default(),
            )
            .unwrap();

        assert!(
            config["outbounds"][0]["streamSettings"]["wsSettings"]
                .get("heartbeatPeriod")
                .is_none()
        );
    }

    #[test]
    fn test_non_ws_outbound_untouched_by_host_migration() {
        let config = XrayGenerator
            .generate(&[xray_vless_with_xtls()], &[], &AppSettings::default())
            .unwrap();

        let stream = &config["outbounds"][0]["streamSettings"];
        assert_eq!(stream["network"], "tcp");
        assert!(stream.get("wsSettings").is_none());
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
