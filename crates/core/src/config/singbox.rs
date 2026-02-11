use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{json, Value};

use crate::config::{ConfigError, ConfigGenerator};
use crate::models::{
    AppSettings, GrpcSettings, H2Settings, ProxyNode, RuleAction, RuleMatch, RoutingRule,
    ShadowsocksConfig, TransportSettings, TrojanConfig, VlessConfig, VmessConfig, WsSettings,
};

pub struct SingboxGenerator;

impl ConfigGenerator for SingboxGenerator {
    fn generate(
        &self,
        nodes: &[ProxyNode],
        rules: &[RoutingRule],
        settings: &AppSettings,
        geodata_dir: Option<&Path>,
    ) -> Result<Value, ConfigError> {
        if nodes.is_empty() {
            return Err(ConfigError::NoNodes);
        }
        Ok(assemble(nodes, rules, settings, geodata_dir))
    }
}

fn assemble(
    nodes: &[ProxyNode],
    rules: &[RoutingRule],
    settings: &AppSettings,
    geodata_dir: Option<&Path>,
) -> Value {
    let inbounds = build_inbounds(settings);
    let outbounds = build_outbounds(nodes);
    let route = build_route(rules, geodata_dir);

    json!({
        "log": { "level": "warn" },
        "inbounds": inbounds,
        "outbounds": outbounds,
        "route": route,
    })
}

fn build_inbounds(settings: &AppSettings) -> Value {
    json!([{
        "type": "mixed",
        "tag": "mixed-in",
        "listen": "127.0.0.1",
        "listen_port": settings.socks_port,
    }])
}

fn build_outbounds(nodes: &[ProxyNode]) -> Value {
    let mut outbounds: Vec<Value> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let tag = outbound_tag(node, i);
            build_outbound(node, &tag)
        })
        .collect();

    outbounds.push(json!({
        "type": "direct",
        "tag": "direct",
    }));
    outbounds.push(json!({
        "type": "block",
        "tag": "block",
    }));

    Value::Array(outbounds)
}

fn outbound_tag(node: &ProxyNode, index: usize) -> String {
    match node.remark() {
        Some(name) if !name.is_empty() => format!("proxy-{index}-{name}"),
        _ => format!("proxy-{index}"),
    }
}

fn build_outbound(node: &ProxyNode, tag: &str) -> Value {
    match node {
        ProxyNode::Vless(c) => build_vless(c, tag),
        ProxyNode::Vmess(c) => build_vmess(c, tag),
        ProxyNode::Shadowsocks(c) => build_ss(c, tag),
        ProxyNode::Trojan(c) => build_trojan(c, tag),
    }
}

fn build_vless(c: &VlessConfig, tag: &str) -> Value {
    let mut out = json!({
        "type": "vless",
        "tag": tag,
        "server": c.address,
        "server_port": c.port,
        "uuid": c.uuid,
    });

    if let Some(flow) = &c.flow {
        out["flow"] = json!(flow);
    }

    apply_transport(&mut out, &c.transport);
    apply_tls(&mut out, c.tls.as_ref());
    out
}

fn build_vmess(c: &VmessConfig, tag: &str) -> Value {
    let mut out = json!({
        "type": "vmess",
        "tag": tag,
        "server": c.address,
        "server_port": c.port,
        "uuid": c.uuid,
        "alter_id": c.alter_id,
        "security": c.security,
    });

    apply_transport(&mut out, &c.transport);
    apply_tls(&mut out, c.tls.as_ref());
    out
}

fn build_ss(c: &ShadowsocksConfig, tag: &str) -> Value {
    json!({
        "type": "shadowsocks",
        "tag": tag,
        "server": c.address,
        "server_port": c.port,
        "method": c.method,
        "password": c.password,
    })
}

fn build_trojan(c: &TrojanConfig, tag: &str) -> Value {
    let mut out = json!({
        "type": "trojan",
        "tag": tag,
        "server": c.address,
        "server_port": c.port,
        "password": c.password,
    });

    apply_transport(&mut out, &c.transport);
    apply_tls(&mut out, c.tls.as_ref());
    out
}

fn apply_transport(out: &mut Value, transport: &TransportSettings) {
    match transport {
        TransportSettings::Tcp => {}
        TransportSettings::Ws(ws) => {
            out["transport"] = build_ws_transport(ws);
        }
        TransportSettings::Grpc(grpc) => {
            out["transport"] = build_grpc_transport(grpc);
        }
        TransportSettings::H2(h2) => {
            out["transport"] = build_h2_transport(h2);
        }
    }
}

fn build_ws_transport(ws: &WsSettings) -> Value {
    let mut transport = json!({
        "type": "ws",
        "path": ws.path,
    });
    if let Some(host) = &ws.host {
        transport["headers"] = json!({ "Host": host });
    }
    if !ws.headers.is_empty() {
        transport["headers"] = json!(ws.headers);
    }
    transport
}

fn build_grpc_transport(grpc: &GrpcSettings) -> Value {
    json!({
        "type": "grpc",
        "service_name": grpc.service_name,
    })
}

fn build_h2_transport(h2: &H2Settings) -> Value {
    json!({
        "type": "http",
        "host": h2.host,
        "path": h2.path,
    })
}

fn apply_tls(out: &mut Value, tls: Option<&crate::models::TlsSettings>) {
    let Some(tls_cfg) = tls else { return };

    let mut tls_obj = json!({
        "enabled": true,
    });

    if let Some(sni) = &tls_cfg.server_name {
        tls_obj["server_name"] = json!(sni);
    }
    if !tls_cfg.alpn.is_empty() {
        tls_obj["alpn"] = json!(tls_cfg.alpn);
    }
    if !tls_cfg.verify {
        tls_obj["insecure"] = json!(true);
    }

    out["tls"] = tls_obj;
}

fn build_route(rules: &[RoutingRule], _geodata_dir: Option<&Path>) -> Value {
    let enabled: Vec<&RoutingRule> = rules.iter().filter(|r| r.enabled).collect();

    if enabled.is_empty() {
        return json!({ "rules": [] });
    }

    let mut geoip_tags = BTreeSet::new();
    let mut geosite_tags = BTreeSet::new();

    for rule in &enabled {
        match &rule.match_condition {
            RuleMatch::GeoIp { country_code } => {
                geoip_tags.insert(country_code.to_lowercase());
            }
            RuleMatch::GeoSite { category } => {
                geosite_tags.insert(category.to_lowercase());
            }
            _ => {}
        }
    }

    let mut rule_sets: Vec<Value> = Vec::new();

    for tag in &geoip_tags {
        rule_sets.push(json!({
            "type": "remote",
            "tag": format!("geoip-{tag}"),
            "format": "binary",
            "url": format!(
                "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-{tag}.srs"
            ),
            "download_detour": "direct",
        }));
    }
    for tag in &geosite_tags {
        rule_sets.push(json!({
            "type": "remote",
            "tag": format!("geosite-{tag}"),
            "format": "binary",
            "url": format!(
                "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-{tag}.srs"
            ),
            "download_detour": "direct",
        }));
    }

    let route_rules: Vec<Value> = enabled.iter().map(|r| build_route_rule(r)).collect();

    if rule_sets.is_empty() {
        json!({ "rules": route_rules })
    } else {
        json!({
            "rule_set": rule_sets,
            "rules": route_rules,
        })
    }
}

fn build_route_rule(rule: &RoutingRule) -> Value {
    let outbound = match rule.action {
        RuleAction::Proxy => "proxy-0",
        RuleAction::Direct => "direct",
        RuleAction::Block => "block",
    };

    match &rule.match_condition {
        RuleMatch::GeoIp { country_code } => json!({
            "rule_set": [format!("geoip-{}", country_code.to_lowercase())],
            "outbound": outbound,
        }),
        RuleMatch::GeoSite { category } => json!({
            "rule_set": [format!("geosite-{}", category.to_lowercase())],
            "outbound": outbound,
        }),
        RuleMatch::Domain { pattern } => json!({
            "domain_suffix": [pattern],
            "outbound": outbound,
        }),
        RuleMatch::IpCidr { cidr } => json!({
            "ip_cidr": [cidr.to_string()],
            "outbound": outbound,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;

    fn default_settings() -> AppSettings {
        AppSettings::default()
    }

    fn vless_node() -> ProxyNode {
        ProxyNode::Vless(VlessConfig {
            address: "example.com".into(),
            port: 443,
            uuid: "test-uuid".into(),
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

    fn ss_node() -> ProxyNode {
        ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: "ss.example.com".into(),
            port: 8388,
            method: "aes-256-gcm".into(),
            password: "secret".into(),
            remark: Some("Test SS".into()),
        })
    }

    fn trojan_node() -> ProxyNode {
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
    fn test_singbox_error_on_empty() {
        let generator = SingboxGenerator;
        assert!(generator.generate(&[], &[], &default_settings(), None).is_err());
    }

    #[test]
    fn test_singbox_basic_structure() {
        let generator = SingboxGenerator;
        let config = generator
            .generate(&[ss_node()], &[], &default_settings(), None)
            .unwrap();

        assert!(config["log"].is_object());
        assert!(config["inbounds"].is_array());
        assert!(config["outbounds"].is_array());
        assert!(config["route"].is_object());
    }

    #[test]
    fn test_singbox_mixed_inbound() {
        let generator = SingboxGenerator;
        let config = generator
            .generate(&[ss_node()], &[], &default_settings(), None)
            .unwrap();

        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 1);
        assert_eq!(inbounds[0]["type"], "mixed");
        assert_eq!(inbounds[0]["listen_port"], 1080);
    }

    #[test]
    fn test_singbox_ss_outbound() {
        let generator = SingboxGenerator;
        let config = generator
            .generate(&[ss_node()], &[], &default_settings(), None)
            .unwrap();

        let out = &config["outbounds"][0];
        assert_eq!(out["type"], "shadowsocks");
        assert_eq!(out["server"], "ss.example.com");
        assert_eq!(out["method"], "aes-256-gcm");
    }

    #[test]
    fn test_singbox_vless_with_ws_tls() {
        let generator = SingboxGenerator;
        let config = generator
            .generate(&[vless_node()], &[], &default_settings(), None)
            .unwrap();

        let out = &config["outbounds"][0];
        assert_eq!(out["type"], "vless");
        assert_eq!(out["transport"]["type"], "ws");
        assert_eq!(out["transport"]["path"], "/ws");
        assert_eq!(out["tls"]["enabled"], true);
        assert_eq!(out["tls"]["server_name"], "example.com");
    }

    #[test]
    fn test_singbox_trojan_outbound() {
        let generator = SingboxGenerator;
        let config = generator
            .generate(&[trojan_node()], &[], &default_settings(), None)
            .unwrap();

        let out = &config["outbounds"][0];
        assert_eq!(out["type"], "trojan");
        assert_eq!(out["password"], "trojan-pass");
        assert_eq!(out["tls"]["enabled"], true);
    }

    #[test]
    fn test_singbox_direct_block_outbounds() {
        let generator = SingboxGenerator;
        let config = generator
            .generate(&[ss_node()], &[], &default_settings(), None)
            .unwrap();

        let outbounds = config["outbounds"].as_array().unwrap();
        let tags: Vec<&str> = outbounds
            .iter()
            .map(|o| o["tag"].as_str().unwrap())
            .collect();
        assert!(tags.contains(&"direct"));
        assert!(tags.contains(&"block"));
    }

    #[test]
    fn test_singbox_geoip_route() {
        let generator = SingboxGenerator;
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: "RU".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
        }];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings(), None)
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert_eq!(route_rules.len(), 1);
        assert_eq!(route_rules[0]["rule_set"][0], "geoip-ru");
        assert_eq!(route_rules[0]["outbound"], "direct");

        let rule_sets = config["route"]["rule_set"].as_array().unwrap();
        assert_eq!(rule_sets.len(), 1);
        assert_eq!(rule_sets[0]["type"], "remote");
        assert_eq!(rule_sets[0]["tag"], "geoip-ru");
        assert_eq!(rule_sets[0]["format"], "binary");
        assert!(rule_sets[0]["url"].as_str().unwrap().contains("geoip-ru.srs"));
        assert_eq!(rule_sets[0]["download_detour"], "direct");
    }

    #[test]
    fn test_singbox_geosite_route() {
        let generator = SingboxGenerator;
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::GeoSite {
                category: "google".into(),
            },
            action: RuleAction::Proxy,
            enabled: true,
        }];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings(), None)
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert_eq!(route_rules[0]["rule_set"][0], "geosite-google");
    }

    #[test]
    fn test_singbox_multiple_nodes() {
        let generator = SingboxGenerator;
        let nodes = vec![vless_node(), ss_node(), trojan_node()];
        let config = generator.generate(&nodes, &[], &default_settings(), None).unwrap();

        let outbounds = config["outbounds"].as_array().unwrap();
        // 3 proxy + direct + block = 5
        assert_eq!(outbounds.len(), 5);
    }

    #[test]
    fn test_singbox_disabled_rules_excluded() {
        let generator = SingboxGenerator;
        let rules = vec![
            RoutingRule {
                id: uuid::Uuid::new_v4(),
                match_condition: RuleMatch::GeoIp {
                    country_code: "RU".into(),
                },
                action: RuleAction::Direct,
                enabled: false,
            },
            RoutingRule {
                id: uuid::Uuid::new_v4(),
                match_condition: RuleMatch::GeoSite {
                    category: "google".into(),
                },
                action: RuleAction::Proxy,
                enabled: true,
            },
        ];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings(), None)
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert_eq!(route_rules.len(), 1);
    }

    #[test]
    fn test_singbox_valid_json() {
        let generator = SingboxGenerator;
        let nodes = vec![vless_node(), ss_node(), trojan_node()];
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: "RU".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
        }];

        let config = generator.generate(&nodes, &rules, &default_settings(), None).unwrap();
        let json_str = serde_json::to_string_pretty(&config).unwrap();
        let _: Value = serde_json::from_str(&json_str).unwrap();
    }
}
