use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::config::{ConfigError, ConfigGenerator};
use crate::models::{
    AppSettings, DnsProtocol, DnsRuleMatch, DnsStrategy, GrpcSettings, H2Settings, ProxyNode,
    RoutingRule, RuleAction, RuleMatch, ShadowsocksConfig, TransportSettings, TrojanConfig,
    VlessConfig, VmessConfig, WsSettings,
};

const GEOIP_RULESET_URL: &str = "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set";
const GEOSITE_RULESET_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set";

pub struct SingboxGenerator;

impl ConfigGenerator for SingboxGenerator {
    fn generate(
        &self,
        nodes: &[ProxyNode],
        rules: &[RoutingRule],
        settings: &AppSettings,
    ) -> Result<Value, ConfigError> {
        if nodes.is_empty() {
            return Err(ConfigError::NoNodes);
        }
        Ok(assemble(nodes, rules, settings))
    }
}

fn assemble(nodes: &[ProxyNode], rules: &[RoutingRule], settings: &AppSettings) -> Value {
    let first_proxy_tag = super::common::outbound_tag(&nodes[0], 0);
    let mut config = json!({
        "log": { "level": "warn" },
        "inbounds": build_inbounds(settings),
        "outbounds": build_outbounds(nodes),
        "route": build_route(rules, &first_proxy_tag),
    });

    if settings.dns.enabled {
        config["dns"] = build_dns(rules, settings);
    }

    config
}

fn build_inbounds(settings: &AppSettings) -> Value {
    json!([
        {
            "type": "mixed",
            "tag": "mixed-in",
            "listen": settings.listen_address,
            "listen_port": settings.socks_port,
        },
        {
            "type": "http",
            "tag": "http-in",
            "listen": settings.listen_address,
            "listen_port": settings.http_port,
        }
    ])
}

fn build_outbounds(nodes: &[ProxyNode]) -> Value {
    let mut outbounds: Vec<Value> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let tag = super::common::outbound_tag(node, i);
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
    let mut headers = ws.headers.clone();
    if let Some(host) = &ws.host {
        headers
            .entry("Host".to_string())
            .or_insert_with(|| host.clone());
    }
    if !headers.is_empty() {
        transport["headers"] = json!(headers);
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

    if tls_cfg.reality {
        let mut reality_obj = json!({ "enabled": true });
        if let Some(pbk) = &tls_cfg.public_key {
            reality_obj["public_key"] = json!(pbk);
        }
        if let Some(sid) = &tls_cfg.short_id {
            reality_obj["short_id"] = json!(sid);
        }
        tls_obj["reality"] = reality_obj;
    }

    if let Some(fp) = &tls_cfg.fingerprint {
        tls_obj["utls"] = json!({ "enabled": true, "fingerprint": fp });
    }

    out["tls"] = tls_obj;
}

fn build_dns(rules: &[RoutingRule], settings: &AppSettings) -> Value {
    let mut dns_config = json!({});

    let strategy_str = match settings.dns.strategy {
        DnsStrategy::PreferIpv4 => "prefer_ipv4",
        DnsStrategy::PreferIpv6 => "prefer_ipv6",
        DnsStrategy::Ipv4Only => "ipv4_only",
        DnsStrategy::Ipv6Only => "ipv6_only",
    };
    dns_config["strategy"] = json!(strategy_str);

    let mut servers: Vec<Value> = Vec::new();

    if !settings.dns.hosts.is_empty() {
        let mut entries = serde_json::Map::new();
        for host in &settings.dns.hosts {
            entries
                .entry(host.domain.clone())
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .unwrap()
                .push(json!(host.ip));
        }
        servers.push(json!({
            "tag": "hosts",
            "type": "hosts",
            "entries": entries,
        }));
    }

    for server_cfg in &settings.dns.servers {
        let port = server_cfg
            .port
            .unwrap_or_else(|| server_cfg.protocol.default_port());

        let mut server = match server_cfg.protocol {
            DnsProtocol::Udp => json!({
                "tag": server_cfg.tag,
                "type": "udp",
                "server": server_cfg.address,
                "server_port": port,
            }),
            DnsProtocol::Tcp => json!({
                "tag": server_cfg.tag,
                "type": "tcp",
                "server": server_cfg.address,
                "server_port": port,
            }),
            DnsProtocol::Doh => json!({
                "tag": server_cfg.tag,
                "type": "https",
                "server": server_cfg.address,
                "server_port": port,
                "path": ["/dns-query"],
            }),
            DnsProtocol::Dot => json!({
                "tag": server_cfg.tag,
                "type": "tls",
                "server": server_cfg.address,
                "server_port": port,
            }),
            DnsProtocol::Doq => json!({
                "tag": server_cfg.tag,
                "type": "quic",
                "server": server_cfg.address,
                "server_port": port,
            }),
            DnsProtocol::H3 => json!({
                "tag": server_cfg.tag,
                "type": "h3",
                "server": server_cfg.address,
                "server_port": port,
            }),
        };

        if let Some(ref detour) = server_cfg.detour {
            server["detour"] = json!(detour);
        }

        if let Some(ref client_subnet) = settings.dns.client_subnet {
            server["client_subnet"] = json!(client_subnet);
        }

        servers.push(server);
    }

    let mut final_server = None;
    if settings.dns.fakeip.enabled {
        servers.push(json!({
            "tag": "fakeip",
            "type": "fakeip",
        }));

        dns_config["fakeip"] = json!({
            "enabled": true,
            "inet4_range": settings.dns.fakeip.inet4_range,
            "inet6_range": settings.dns.fakeip.inet6_range,
        });

        final_server = Some("fakeip".to_string());
    }

    let final_server = final_server.or_else(|| settings.dns.servers.first().map(|s| s.tag.clone()));
    dns_config["final"] = json!(final_server);

    dns_config["servers"] = json!(servers);

    let dns_rules = if settings.dns.use_custom_rules {
        settings
            .dns
            .rules
            .iter()
            .map(|rule| match &rule.match_condition {
                DnsRuleMatch::GeoSite { category } => json!({
                    "rule_set": [format!("geosite-{category}")],
                    "server": rule.server_tag,
                }),
                DnsRuleMatch::DomainSuffix { suffix } => json!({
                    "domain_suffix": [suffix],
                    "server": rule.server_tag,
                }),
            })
            .collect()
    } else {
        let mut remote_geosite: Vec<String> = Vec::new();
        let mut domestic_geosite: Vec<String> = Vec::new();
        let mut remote_domains: Vec<String> = Vec::new();
        let mut domestic_domains: Vec<String> = Vec::new();

        for rule in rules.iter().filter(|r| r.enabled) {
            match &rule.match_condition {
                RuleMatch::GeoSite { category } => {
                    let tag = format!("geosite-{category}");
                    match rule.action {
                        RuleAction::Proxy => remote_geosite.push(tag),
                        RuleAction::Direct => domestic_geosite.push(tag),
                        RuleAction::Block => {}
                    }
                }
                RuleMatch::Domain { pattern } => match rule.action {
                    RuleAction::Proxy => remote_domains.push(pattern.clone()),
                    RuleAction::Direct => domestic_domains.push(pattern.clone()),
                    RuleAction::Block => {}
                },
                _ => {}
            }
        }

        let mut derived_rules: Vec<Value> = Vec::new();
        if !remote_geosite.is_empty() {
            derived_rules.push(json!({ "rule_set": remote_geosite, "server": "remote" }));
        }
        if !remote_domains.is_empty() {
            derived_rules.push(json!({ "domain_suffix": remote_domains, "server": "remote" }));
        }
        if !domestic_geosite.is_empty() {
            derived_rules.push(json!({ "rule_set": domestic_geosite, "server": "domestic" }));
        }
        if !domestic_domains.is_empty() {
            derived_rules.push(json!({ "domain_suffix": domestic_domains, "server": "domestic" }));
        }

        derived_rules
    };

    dns_config["rules"] = json!(dns_rules);

    if settings.dns.disable_cache {
        dns_config["disable_cache"] = json!(true);
    }

    dns_config
}

fn build_route(rules: &[RoutingRule], first_proxy_tag: &str) -> Value {
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
            "url": format!("{GEOIP_RULESET_URL}/geoip-{tag}.srs"),
            "download_detour": "direct",
        }));
    }
    for tag in &geosite_tags {
        rule_sets.push(json!({
            "type": "remote",
            "tag": format!("geosite-{tag}"),
            "format": "binary",
            "url": format!("{GEOSITE_RULESET_URL}/geosite-{tag}.srs"),
            "download_detour": "direct",
        }));
    }

    let route_rules: Vec<Value> = enabled
        .iter()
        .map(|r| build_route_rule(r, first_proxy_tag))
        .collect();

    if rule_sets.is_empty() {
        json!({ "rules": route_rules })
    } else {
        json!({
            "rule_set": rule_sets,
            "rules": route_rules,
        })
    }
}

fn build_route_rule(rule: &RoutingRule, first_proxy_tag: &str) -> Value {
    let outbound = match rule.action {
        RuleAction::Proxy => first_proxy_tag,
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
    use crate::config::test_fixtures::fixtures::*;
    use crate::models::*;

    #[test]
    fn test_singbox_error_on_empty() {
        let generator = SingboxGenerator;
        assert!(generator.generate(&[], &[], &default_settings()).is_err());
    }

    #[test]
    fn test_singbox_basic_structure() {
        let generator = SingboxGenerator;
        let config = generator
            .generate(&[ss_node()], &[], &default_settings())
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
            .generate(&[ss_node()], &[], &default_settings())
            .unwrap();

        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 2);
        assert_eq!(inbounds[0]["type"], "mixed");
        assert_eq!(inbounds[0]["listen_port"], 1080);
        assert_eq!(inbounds[1]["type"], "http");
        assert_eq!(inbounds[1]["listen_port"], 1081);
    }

    #[test]
    fn test_singbox_inbound_listen_address_default_loopback() {
        let generator = SingboxGenerator;
        let config = generator
            .generate(&[ss_node()], &[], &default_settings())
            .unwrap();

        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds[0]["listen"], "127.0.0.1");
        assert_eq!(inbounds[1]["listen"], "127.0.0.1");
    }

    #[test]
    fn test_singbox_inbound_listen_address_from_settings() {
        let generator = SingboxGenerator;
        let mut settings = default_settings();
        settings.listen_address = "192.168.1.10".to_string();
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds[0]["listen"], "192.168.1.10");
        assert_eq!(inbounds[1]["listen"], "192.168.1.10");
        // ports unchanged
        assert_eq!(inbounds[0]["listen_port"], 1080);
        assert_eq!(inbounds[1]["listen_port"], 1081);
    }

    #[test]
    fn test_singbox_mixed_inbound_udp_enabled() {
        let generator = SingboxGenerator;
        let config = generator
            .generate(&[ss_node()], &[], &default_settings())
            .unwrap();

        assert_eq!(config["inbounds"][0]["type"], "mixed");
        // sing-box `mixed` inbound supports UDP implicitly; assert we did not
        // emit `udp_disabled: true` which would silently break UDP-over-SOCKS.
        let disabled = config["inbounds"][0].get("udp_disabled");
        assert!(
            !matches!(disabled, Some(v) if v == &json!(true)),
            "udp_disabled must not be true on mixed inbound, got {:?}",
            disabled
        );
    }

    #[test]
    fn test_singbox_ss_outbound() {
        let generator = SingboxGenerator;
        let config = generator
            .generate(&[ss_node()], &[], &default_settings())
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
            .generate(&[vless_node()], &[], &default_settings())
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
            .generate(&[trojan_node()], &[], &default_settings())
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
            .generate(&[ss_node()], &[], &default_settings())
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
            group: None,
        }];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings())
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
        assert!(
            rule_sets[0]["url"]
                .as_str()
                .unwrap()
                .contains("geoip-ru.srs")
        );
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
            group: None,
        }];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings())
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert_eq!(route_rules[0]["rule_set"][0], "geosite-google");
    }

    #[test]
    fn test_singbox_multiple_nodes() {
        let generator = SingboxGenerator;
        let nodes = vec![vless_node(), ss_node(), trojan_node()];
        let config = generator
            .generate(&nodes, &[], &default_settings())
            .unwrap();

        let outbounds = config["outbounds"].as_array().unwrap();
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
                group: None,
            },
            RoutingRule {
                id: uuid::Uuid::new_v4(),
                match_condition: RuleMatch::GeoSite {
                    category: "google".into(),
                },
                action: RuleAction::Proxy,
                enabled: true,
                group: None,
            },
        ];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings())
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert_eq!(route_rules.len(), 1);
    }

    #[test]
    fn test_dns_udp_server() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "dns-udp".to_string(),
            protocol: DnsProtocol::Udp,
            address: "8.8.8.8".to_string(),
            port: None,
            detour: None,
        }];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let servers = config["dns"]["servers"].as_array().unwrap();
        let udp_server = servers.iter().find(|s| s["tag"] == "dns-udp").unwrap();
        assert_eq!(udp_server["type"], "udp");
        assert_eq!(udp_server["server"], "8.8.8.8");
        assert_eq!(udp_server["server_port"], 53);
    }

    #[test]
    fn test_dns_tcp_server() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "dns-tcp".to_string(),
            protocol: DnsProtocol::Tcp,
            address: "8.8.8.8".to_string(),
            port: Some(5353),
            detour: None,
        }];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let servers = config["dns"]["servers"].as_array().unwrap();
        let tcp_server = servers.iter().find(|s| s["tag"] == "dns-tcp").unwrap();
        assert_eq!(tcp_server["type"], "tcp");
        assert_eq!(tcp_server["server"], "8.8.8.8");
        assert_eq!(tcp_server["server_port"], 5353);
    }

    #[test]
    fn test_dns_doh_server() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "dns-doh".to_string(),
            protocol: DnsProtocol::Doh,
            address: "cloudflare-dns.com".to_string(),
            port: None,
            detour: None,
        }];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let servers = config["dns"]["servers"].as_array().unwrap();
        let doh_server = servers.iter().find(|s| s["tag"] == "dns-doh").unwrap();
        assert_eq!(doh_server["type"], "https");
        assert_eq!(doh_server["server"], "cloudflare-dns.com");
        assert_eq!(doh_server["server_port"], 443);
        assert_eq!(doh_server["path"], json!(["/dns-query"]));
    }

    #[test]
    fn test_dns_dot_server() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "dns-dot".to_string(),
            protocol: DnsProtocol::Dot,
            address: "dns.google".to_string(),
            port: None,
            detour: None,
        }];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let servers = config["dns"]["servers"].as_array().unwrap();
        let dot_server = servers.iter().find(|s| s["tag"] == "dns-dot").unwrap();
        assert_eq!(dot_server["type"], "tls");
        assert_eq!(dot_server["server"], "dns.google");
        assert_eq!(dot_server["server_port"], 853);
    }

    #[test]
    fn test_dns_doq_server() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "dns-doq".to_string(),
            protocol: DnsProtocol::Doq,
            address: "dns.adguard.com".to_string(),
            port: None,
            detour: None,
        }];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let servers = config["dns"]["servers"].as_array().unwrap();
        let doq_server = servers.iter().find(|s| s["tag"] == "dns-doq").unwrap();
        assert_eq!(doq_server["type"], "quic");
        assert_eq!(doq_server["server"], "dns.adguard.com");
        assert_eq!(doq_server["server_port"], 853);
    }

    #[test]
    fn test_dns_h3_server() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "dns-h3".to_string(),
            protocol: DnsProtocol::H3,
            address: "dns.google".to_string(),
            port: None,
            detour: None,
        }];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let servers = config["dns"]["servers"].as_array().unwrap();
        let h3_server = servers.iter().find(|s| s["tag"] == "dns-h3").unwrap();
        assert_eq!(h3_server["type"], "h3");
        assert_eq!(h3_server["server"], "dns.google");
        assert_eq!(h3_server["server_port"], 443);
    }

    #[test]
    fn test_dns_strategy_mapping() {
        let test_cases = [
            (DnsStrategy::PreferIpv4, "prefer_ipv4"),
            (DnsStrategy::PreferIpv6, "prefer_ipv6"),
            (DnsStrategy::Ipv4Only, "ipv4_only"),
            (DnsStrategy::Ipv6Only, "ipv6_only"),
        ];

        for (strategy, expected_str) in test_cases {
            let mut settings = default_settings();
            settings.dns.enabled = true;
            settings.dns.strategy = strategy;

            let generator = SingboxGenerator;
            let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

            assert_eq!(config["dns"]["strategy"], expected_str);
        }
    }

    #[test]
    fn test_dns_fakeip_config() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.fakeip = FakeIpConfig {
            enabled: true,
            inet4_range: "198.18.0.0/16".to_string(),
            inet6_range: "fc00::/16".to_string(),
        };

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        assert_eq!(config["dns"]["fakeip"]["enabled"], true);
        assert_eq!(config["dns"]["fakeip"]["inet4_range"], "198.18.0.0/16");
        assert_eq!(config["dns"]["fakeip"]["inet6_range"], "fc00::/16");

        let servers = config["dns"]["servers"].as_array().unwrap();
        let fakeip_server = servers.iter().find(|s| s["tag"] == "fakeip").unwrap();
        assert_eq!(fakeip_server["type"], "fakeip");

        assert_eq!(config["dns"]["final"], "fakeip");
    }

    #[test]
    fn test_dns_hosts_server() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.hosts = vec![
            HostOverride {
                domain: "example.com".to_string(),
                ip: "192.0.2.1".to_string(),
            },
            HostOverride {
                domain: "test.local".to_string(),
                ip: "192.0.2.2".to_string(),
            },
        ];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let servers = config["dns"]["servers"].as_array().unwrap();
        let hosts_server = servers.iter().find(|s| s["tag"] == "hosts").unwrap();

        assert_eq!(hosts_server["type"], "hosts");
        assert_eq!(hosts_server["entries"]["example.com"], json!(["192.0.2.1"]));
        assert_eq!(hosts_server["entries"]["test.local"], json!(["192.0.2.2"]));
    }

    #[test]
    fn test_dns_custom_rules() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.use_custom_rules = true;
        settings.dns.rules = vec![
            DnsRule {
                match_condition: DnsRuleMatch::GeoSite {
                    category: "cn".to_string(),
                },
                server_tag: "remote".to_string(),
            },
            DnsRule {
                match_condition: DnsRuleMatch::DomainSuffix {
                    suffix: ".google.com".to_string(),
                },
                server_tag: "domestic".to_string(),
            },
        ];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let rules = config["dns"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 2);

        assert_eq!(rules[0]["rule_set"], json!(["geosite-cn"]));
        assert_eq!(rules[0]["server"], "remote");

        assert_eq!(rules[1]["domain_suffix"], json!([".google.com"]));
        assert_eq!(rules[1]["server"], "domestic");
    }

    #[test]
    fn test_dns_disable_cache() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.disable_cache = true;

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        assert_eq!(config["dns"]["disable_cache"], true);
    }

    #[test]
    fn test_dns_client_subnet() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.client_subnet = Some("203.0.113.1".to_string());

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let servers = config["dns"]["servers"].as_array().unwrap();
        for server in servers {
            if server["type"] != "hosts" && server["type"] != "fakeip" {
                assert_eq!(server["client_subnet"], "203.0.113.1");
            }
        }
    }

    #[test]
    fn test_dns_server_detour() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "proxy-dns".to_string(),
            protocol: DnsProtocol::Doh,
            address: "cloudflare-dns.com".to_string(),
            port: None,
            detour: Some("proxy-0".to_string()),
        }];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let servers = config["dns"]["servers"].as_array().unwrap();
        let proxy_dns = servers.iter().find(|s| s["tag"] == "proxy-dns").unwrap();
        assert_eq!(proxy_dns["detour"], "proxy-0");
    }

    #[test]
    fn test_dns_auto_derive_rules_from_routing() {
        let rules = vec![
            RoutingRule {
                id: uuid::Uuid::new_v4(),
                match_condition: RuleMatch::GeoSite {
                    category: "google".to_string(),
                },
                action: RuleAction::Proxy,
                enabled: true,
                group: None,
            },
            RoutingRule {
                id: uuid::Uuid::new_v4(),
                match_condition: RuleMatch::GeoSite {
                    category: "cn".to_string(),
                },
                action: RuleAction::Direct,
                enabled: true,
                group: None,
            },
            RoutingRule {
                id: uuid::Uuid::new_v4(),
                match_condition: RuleMatch::Domain {
                    pattern: ".example.com".to_string(),
                },
                action: RuleAction::Proxy,
                enabled: true,
                group: None,
            },
        ];

        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.use_custom_rules = false;

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &rules, &settings).unwrap();

        let dns_rules = config["dns"]["rules"].as_array().unwrap();
        assert!(!dns_rules.is_empty());
    }

    #[test]
    fn test_dns_full_config_valid_json() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.strategy = DnsStrategy::Ipv6Only;
        settings.dns.servers = vec![
            DnsServerConfig {
                tag: "cloudflare".to_string(),
                protocol: DnsProtocol::Doh,
                address: "1.1.1.1".to_string(),
                port: None,
                detour: Some("proxy-0".to_string()),
            },
            DnsServerConfig {
                tag: "google".to_string(),
                protocol: DnsProtocol::Doq,
                address: "dns.adguard.com".to_string(),
                port: Some(784),
                detour: None,
            },
        ];
        settings.dns.use_custom_rules = true;
        settings.dns.rules = vec![
            DnsRule {
                match_condition: DnsRuleMatch::GeoSite {
                    category: "google".to_string(),
                },
                server_tag: "cloudflare".to_string(),
            },
            DnsRule {
                match_condition: DnsRuleMatch::DomainSuffix {
                    suffix: ".cn".to_string(),
                },
                server_tag: "google".to_string(),
            },
        ];
        settings.dns.fakeip = FakeIpConfig {
            enabled: true,
            inet4_range: "198.18.0.0/16".to_string(),
            inet6_range: "fc00::/16".to_string(),
        };
        settings.dns.disable_cache = true;
        settings.dns.client_subnet = Some("203.0.113.1".to_string());
        settings.dns.hosts = vec![
            HostOverride {
                domain: "example.com".to_string(),
                ip: "192.0.2.1".to_string(),
            },
            HostOverride {
                domain: "test.local".to_string(),
                ip: "10.0.0.1".to_string(),
            },
        ];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        assert!(config.get("dns").is_some());
        let dns = &config["dns"];

        assert_eq!(dns["strategy"], "ipv6_only");
        assert_eq!(dns["disable_cache"], true);

        let servers = dns["servers"].as_array().unwrap();
        assert!(servers.len() >= 3);

        let hosts_server = servers
            .iter()
            .find(|s| s["tag"] == "hosts")
            .expect("hosts server not found");
        assert_eq!(hosts_server["type"], "hosts");
        assert_eq!(hosts_server["entries"]["example.com"], json!(["192.0.2.1"]));
        assert_eq!(hosts_server["entries"]["test.local"], json!(["10.0.0.1"]));

        let fakeip_server = servers
            .iter()
            .find(|s| s["tag"] == "fakeip")
            .expect("fakeip server not found");
        assert_eq!(fakeip_server["type"], "fakeip");

        assert_eq!(dns["fakeip"]["enabled"], true);
        assert_eq!(dns["fakeip"]["inet4_range"], "198.18.0.0/16");
        assert_eq!(dns["fakeip"]["inet6_range"], "fc00::/16");
        assert_eq!(dns["final"], "fakeip");

        let cloudflare = servers
            .iter()
            .find(|s| s["tag"] == "cloudflare")
            .expect("cloudflare server not found");
        assert_eq!(cloudflare["client_subnet"], "203.0.113.1");

        let google = servers
            .iter()
            .find(|s| s["tag"] == "google")
            .expect("google server not found");
        assert_eq!(google["client_subnet"], "203.0.113.1");

        let dns_rules = dns["rules"].as_array().unwrap();
        assert_eq!(dns_rules.len(), 2);

        let json_str = serde_json::to_string(&config).unwrap();
        let _: Value = serde_json::from_str(&json_str).unwrap();
    }
}
