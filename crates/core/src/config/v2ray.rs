use std::path::Path;

use serde_json::{json, Value};

use crate::config::{ConfigError, ConfigGenerator};
use crate::models::{
    AppSettings, GrpcSettings, H2Settings, ProxyNode, RuleAction, RuleMatch, RoutingRule,
    ShadowsocksConfig, TransportSettings, TrojanConfig, VlessConfig, VmessConfig, WsSettings,
};

pub struct V2rayGenerator;

impl ConfigGenerator for V2rayGenerator {
    fn generate(
        &self,
        nodes: &[ProxyNode],
        rules: &[RoutingRule],
        settings: &AppSettings,
        _geodata_dir: Option<&Path>,
    ) -> Result<Value, ConfigError> {
        if nodes.is_empty() {
            return Err(ConfigError::NoNodes);
        }
        Ok(assemble(nodes, rules, settings))
    }
}

fn assemble(nodes: &[ProxyNode], rules: &[RoutingRule], settings: &AppSettings) -> Value {
    let inbounds = build_inbounds(settings);
    let outbounds = build_outbounds(nodes);
    let routing = build_routing(rules);

    json!({
        "log": { "loglevel": "warning" },
        "inbounds": inbounds,
        "outbounds": outbounds,
        "routing": routing,
    })
}

fn build_inbounds(settings: &AppSettings) -> Value {
    json!([
        {
            "tag": "socks-in",
            "protocol": "socks",
            "listen": "127.0.0.1",
            "port": settings.socks_port,
            "settings": { "udp": true },
        },
        {
            "tag": "http-in",
            "protocol": "http",
            "listen": "127.0.0.1",
            "port": settings.http_port,
        },
    ])
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
        "tag": "direct",
        "protocol": "freedom",
        "settings": {},
    }));
    outbounds.push(json!({
        "tag": "block",
        "protocol": "blackhole",
        "settings": {},
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
        ProxyNode::Vless(c) => build_vless_outbound(c, tag),
        ProxyNode::Vmess(c) => build_vmess_outbound(c, tag),
        ProxyNode::Shadowsocks(c) => build_ss_outbound(c, tag),
        ProxyNode::Trojan(c) => build_trojan_outbound(c, tag),
    }
}

fn build_vless_outbound(c: &VlessConfig, tag: &str) -> Value {
    let mut user = json!({
        "id": c.uuid,
        "encryption": c.encryption.as_deref().unwrap_or("none"),
    });
    if let Some(flow) = &c.flow {
        user["flow"] = json!(flow);
    }

    let mut outbound = json!({
        "tag": tag,
        "protocol": "vless",
        "settings": {
            "vnext": [{
                "address": c.address,
                "port": c.port,
                "users": [user],
            }],
        },
    });

    apply_stream_settings(&mut outbound, &c.transport, c.tls.as_ref());
    outbound
}

fn build_vmess_outbound(c: &VmessConfig, tag: &str) -> Value {
    let mut outbound = json!({
        "tag": tag,
        "protocol": "vmess",
        "settings": {
            "vnext": [{
                "address": c.address,
                "port": c.port,
                "users": [{
                    "id": c.uuid,
                    "alterId": c.alter_id,
                    "security": c.security,
                }],
            }],
        },
    });

    apply_stream_settings(&mut outbound, &c.transport, c.tls.as_ref());
    outbound
}

fn build_ss_outbound(c: &ShadowsocksConfig, tag: &str) -> Value {
    json!({
        "tag": tag,
        "protocol": "shadowsocks",
        "settings": {
            "servers": [{
                "address": c.address,
                "port": c.port,
                "method": c.method,
                "password": c.password,
            }],
        },
    })
}

fn build_trojan_outbound(c: &TrojanConfig, tag: &str) -> Value {
    let mut outbound = json!({
        "tag": tag,
        "protocol": "trojan",
        "settings": {
            "servers": [{
                "address": c.address,
                "port": c.port,
                "password": c.password,
            }],
        },
    });

    apply_stream_settings(&mut outbound, &c.transport, c.tls.as_ref());
    outbound
}

fn apply_stream_settings(
    outbound: &mut Value,
    transport: &TransportSettings,
    tls: Option<&crate::models::TlsSettings>,
) {
    let mut stream = json!({});

    match transport {
        TransportSettings::Tcp => {
            stream["network"] = json!("tcp");
        }
        TransportSettings::Ws(ws) => {
            stream["network"] = json!("ws");
            stream["wsSettings"] = build_ws_settings(ws);
        }
        TransportSettings::Grpc(grpc) => {
            stream["network"] = json!("grpc");
            stream["grpcSettings"] = build_grpc_settings(grpc);
        }
        TransportSettings::H2(h2) => {
            stream["network"] = json!("h2");
            stream["httpSettings"] = build_h2_settings(h2);
        }
    }

    if let Some(tls_cfg) = tls {
        stream["security"] = json!("tls");
        let mut tls_obj = json!({});
        if let Some(sni) = &tls_cfg.server_name {
            tls_obj["serverName"] = json!(sni);
        }
        if !tls_cfg.alpn.is_empty() {
            tls_obj["alpn"] = json!(tls_cfg.alpn);
        }
        tls_obj["allowInsecure"] = json!(!tls_cfg.verify);
        if let Some(fp) = &tls_cfg.fingerprint {
            tls_obj["fingerprint"] = json!(fp);
        }
        stream["tlsSettings"] = tls_obj;
    }

    outbound["streamSettings"] = stream;
}

fn build_ws_settings(ws: &WsSettings) -> Value {
    let mut settings = json!({ "path": ws.path });
    if !ws.headers.is_empty() {
        settings["headers"] = json!(ws.headers);
    } else if let Some(host) = &ws.host {
        settings["headers"] = json!({ "Host": host });
    }
    settings
}

fn build_grpc_settings(grpc: &GrpcSettings) -> Value {
    json!({
        "serviceName": grpc.service_name,
        "multiMode": grpc.multi_mode,
    })
}

fn build_h2_settings(h2: &H2Settings) -> Value {
    json!({
        "host": h2.host,
        "path": h2.path,
    })
}

fn build_routing(rules: &[RoutingRule]) -> Value {
    let enabled: Vec<&RoutingRule> = rules.iter().filter(|r| r.enabled).collect();

    if enabled.is_empty() {
        return json!({
            "domainStrategy": "AsIs",
            "rules": [],
        });
    }

    let routing_rules: Vec<Value> = enabled.iter().map(|r| build_routing_rule(r)).collect();

    json!({
        "domainStrategy": "IPIfNonMatch",
        "rules": routing_rules,
    })
}

fn build_routing_rule(rule: &RoutingRule) -> Value {
    let outbound_tag = match rule.action {
        RuleAction::Proxy => first_proxy_tag(),
        RuleAction::Direct => "direct".to_string(),
        RuleAction::Block => "block".to_string(),
    };

    match &rule.match_condition {
        RuleMatch::GeoIp { country_code } => json!({
            "type": "field",
            "ip": [format!("geoip:{}", country_code.to_lowercase())],
            "outboundTag": outbound_tag,
        }),
        RuleMatch::GeoSite { category } => json!({
            "type": "field",
            "domain": [format!("geosite:{}", category.to_lowercase())],
            "outboundTag": outbound_tag,
        }),
        RuleMatch::Domain { pattern } => json!({
            "type": "field",
            "domain": [pattern],
            "outboundTag": outbound_tag,
        }),
        RuleMatch::IpCidr { cidr } => json!({
            "type": "field",
            "ip": [cidr.to_string()],
            "outboundTag": outbound_tag,
        }),
    }
}

fn first_proxy_tag() -> String {
    "proxy-0".to_string()
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

    fn vmess_node() -> ProxyNode {
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
    fn test_generate_returns_error_on_empty_nodes() {
        let generator = V2rayGenerator;
        let result = generator.generate(&[], &[], &default_settings(), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_basic_vless_config_structure() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[vless_node()], &[], &default_settings(), None)
            .unwrap();

        assert!(config["log"].is_object());
        assert!(config["inbounds"].is_array());
        assert!(config["outbounds"].is_array());
        assert!(config["routing"].is_object());
    }

    #[test]
    fn test_inbound_ports() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[vless_node()], &[], &default_settings(), None)
            .unwrap();

        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 2);
        assert_eq!(inbounds[0]["port"], 1080);
        assert_eq!(inbounds[0]["protocol"], "socks");
        assert_eq!(inbounds[1]["port"], 1081);
        assert_eq!(inbounds[1]["protocol"], "http");
    }

    #[test]
    fn test_vless_outbound() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[vless_node()], &[], &default_settings(), None)
            .unwrap();

        let outbounds = config["outbounds"].as_array().unwrap();
        let proxy = &outbounds[0];
        assert_eq!(proxy["protocol"], "vless");
        assert_eq!(
            proxy["settings"]["vnext"][0]["address"],
            "example.com"
        );
        assert_eq!(proxy["settings"]["vnext"][0]["port"], 443);

        let stream = &proxy["streamSettings"];
        assert_eq!(stream["network"], "ws");
        assert_eq!(stream["security"], "tls");
        assert_eq!(stream["wsSettings"]["path"], "/ws");
    }

    #[test]
    fn test_vmess_outbound() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[vmess_node()], &[], &default_settings(), None)
            .unwrap();

        let proxy = &config["outbounds"][0];
        assert_eq!(proxy["protocol"], "vmess");
        assert_eq!(proxy["settings"]["vnext"][0]["users"][0]["security"], "auto");
        assert_eq!(proxy["settings"]["vnext"][0]["users"][0]["alterId"], 0);
    }

    #[test]
    fn test_shadowsocks_outbound() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[ss_node()], &[], &default_settings(), None)
            .unwrap();

        let proxy = &config["outbounds"][0];
        assert_eq!(proxy["protocol"], "shadowsocks");
        assert_eq!(proxy["settings"]["servers"][0]["method"], "aes-256-gcm");
    }

    #[test]
    fn test_trojan_outbound() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[trojan_node()], &[], &default_settings(), None)
            .unwrap();

        let proxy = &config["outbounds"][0];
        assert_eq!(proxy["protocol"], "trojan");
        assert_eq!(proxy["settings"]["servers"][0]["password"], "trojan-pass");
        assert_eq!(proxy["streamSettings"]["security"], "tls");
    }

    #[test]
    fn test_direct_and_block_outbounds_present() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[vless_node()], &[], &default_settings(), None)
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
    fn test_multiple_nodes() {
        let generator = V2rayGenerator;
        let nodes = vec![vless_node(), vmess_node(), ss_node(), trojan_node()];
        let config = generator.generate(&nodes, &[], &default_settings(), None).unwrap();

        let outbounds = config["outbounds"].as_array().unwrap();
        // 4 proxy + direct + block = 6
        assert_eq!(outbounds.len(), 6);
    }

    #[test]
    fn test_geoip_routing_rule() {
        let generator = V2rayGenerator;
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: "RU".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
        }];

        let config = generator
            .generate(&[vless_node()], &rules, &default_settings(), None)
            .unwrap();

        let routing_rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(routing_rules.len(), 1);
        assert_eq!(routing_rules[0]["ip"][0], "geoip:ru");
        assert_eq!(routing_rules[0]["outboundTag"], "direct");
    }

    #[test]
    fn test_geosite_routing_rule() {
        let generator = V2rayGenerator;
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::GeoSite {
                category: "google".into(),
            },
            action: RuleAction::Proxy,
            enabled: true,
        }];

        let config = generator
            .generate(&[vless_node()], &rules, &default_settings(), None)
            .unwrap();

        let routing_rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(routing_rules[0]["domain"][0], "geosite:google");
    }

    #[test]
    fn test_domain_routing_rule() {
        let generator = V2rayGenerator;
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Domain {
                pattern: "*.google.com".into(),
            },
            action: RuleAction::Proxy,
            enabled: true,
        }];

        let config = generator
            .generate(&[vless_node()], &rules, &default_settings(), None)
            .unwrap();

        let routing_rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(routing_rules[0]["domain"][0], "*.google.com");
    }

    #[test]
    fn test_ip_cidr_routing_rule() {
        let generator = V2rayGenerator;
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::IpCidr {
                cidr: "192.168.0.0/16".parse().unwrap(),
            },
            action: RuleAction::Direct,
            enabled: true,
        }];

        let config = generator
            .generate(&[vless_node()], &rules, &default_settings(), None)
            .unwrap();

        let routing_rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(routing_rules[0]["ip"][0], "192.168.0.0/16");
        assert_eq!(routing_rules[0]["outboundTag"], "direct");
    }

    #[test]
    fn test_disabled_rules_excluded() {
        let generator = V2rayGenerator;
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
            .generate(&[vless_node()], &rules, &default_settings(), None)
            .unwrap();

        let routing_rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(routing_rules.len(), 1);
        assert_eq!(routing_rules[0]["domain"][0], "geosite:google");
    }

    #[test]
    fn test_grpc_transport() {
        let node = ProxyNode::Vless(VlessConfig {
            address: "grpc.example.com".into(),
            port: 443,
            uuid: "test-uuid".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Grpc(GrpcSettings {
                service_name: "mygrpc".into(),
                multi_mode: true,
            }),
            tls: None,
            remark: None,
        });

        let generator = V2rayGenerator;
        let config = generator
            .generate(&[node], &[], &default_settings(), None)
            .unwrap();

        let stream = &config["outbounds"][0]["streamSettings"];
        assert_eq!(stream["network"], "grpc");
        assert_eq!(stream["grpcSettings"]["serviceName"], "mygrpc");
        assert_eq!(stream["grpcSettings"]["multiMode"], true);
    }

    #[test]
    fn test_h2_transport() {
        let node = ProxyNode::Vless(VlessConfig {
            address: "h2.example.com".into(),
            port: 443,
            uuid: "test-uuid".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::H2(H2Settings {
                host: vec!["h2.example.com".into()],
                path: "/h2path".into(),
            }),
            tls: None,
            remark: None,
        });

        let generator = V2rayGenerator;
        let config = generator
            .generate(&[node], &[], &default_settings(), None)
            .unwrap();

        let stream = &config["outbounds"][0]["streamSettings"];
        assert_eq!(stream["network"], "h2");
        assert_eq!(stream["httpSettings"]["path"], "/h2path");
    }

    #[test]
    fn test_config_is_valid_json() {
        let generator = V2rayGenerator;
        let nodes = vec![vless_node(), vmess_node(), ss_node(), trojan_node()];
        let rules = vec![
            RoutingRule {
                id: uuid::Uuid::new_v4(),
                match_condition: RuleMatch::GeoIp {
                    country_code: "RU".into(),
                },
                action: RuleAction::Direct,
                enabled: true,
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

        let config = generator.generate(&nodes, &rules, &default_settings(), None).unwrap();
        let json_str = serde_json::to_string_pretty(&config).unwrap();
        let _: Value = serde_json::from_str(&json_str).unwrap();
    }
}
