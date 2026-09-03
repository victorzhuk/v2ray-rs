use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::config::{ConfigError, ConfigGenerator};
use crate::models::{
    AppSettings, BackendType, ConnectionNodeRef, DnsHijackMode, DnsProtocol, DnsRuleMatch,
    DnsStrategy, GrpcSettings, H2Settings, ProxyNode, RoutingRule, RuleAction, RuleMatch,
    ShadowsocksConfig, TransportSettings, TrojanConfig, TunConfig, VlessConfig, VmessConfig,
    WsSettings,
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
        assemble(nodes, rules, settings)
    }
}

fn assemble(
    nodes: &[ProxyNode],
    rules: &[RoutingRule],
    settings: &AppSettings,
) -> Result<Value, ConfigError> {
    let first_proxy_tag = super::common::outbound_tag(&nodes[0], 0);
    let via_tags = super::v2ray::via_outbound_tags(nodes, rules);
    let mut config = json!({
        "log": { "level": "warn" },
        "inbounds": build_inbounds(settings),
        "outbounds": build_outbounds(nodes, settings)?,
        "route": build_route(rules, &first_proxy_tag, &via_tags, settings),
    });

    if settings.dns.enabled {
        config["dns"] = build_dns(rules, settings, &first_proxy_tag);
        if let Some(resolver) = default_domain_resolver_tag(settings) {
            config["route"]["default_domain_resolver"] = json!(resolver);
        }
    } else if settings.tun.enabled {
        // TUN must never depend on the OS resolver: with the DNS feature off,
        // derive a minimal trusted plane so hijack-dns and route resolution
        // have somewhere clean to go (poisoned ISP answers otherwise feed
        // geoip routing and direct dials).
        config["dns"] = derived_tun_dns(settings, &first_proxy_tag);
        config["route"]["default_domain_resolver"] = json!(DERIVED_DNS_TAG);
    }

    if settings.tun.enabled {
        config["route"]["auto_detect_interface"] = json!(true);
    }

    Ok(config)
}

const DERIVED_DNS_TAG: &str = "remote";
const HOSTS_SERVER_TAG: &str = "hosts";

fn derived_tun_dns(settings: &AppSettings, first_proxy_tag: &str) -> Value {
    let mut servers = Vec::new();
    let mut rules = Vec::new();

    // The connect-time pin is the only thing that resolves the proxy's own
    // hostname without the proxy, so it belongs on this path too - not just
    // on the one the DNS feature builds.
    if let Some(hosts) = hosts_server(settings) {
        servers.push(hosts);
        rules.push(hosts_rule(settings));
    }

    servers.push(json!({
        "tag": DERIVED_DNS_TAG,
        "type": "https",
        "server": "1.1.1.1",
        "server_port": 443,
        "path": "/dns-query",
        "detour": first_proxy_tag,
    }));

    json!({
        "strategy": strategy_str(settings.dns.strategy),
        "servers": servers,
        "rules": rules,
        "final": DERIVED_DNS_TAG,
    })
}

fn hosts_server(settings: &AppSettings) -> Option<Value> {
    if settings.dns.hosts.is_empty() {
        return None;
    }

    let mut predefined = serde_json::Map::new();
    for host in &settings.dns.hosts {
        predefined
            .entry(host.domain.clone())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("predefined entries are arrays")
            .push(json!(host.ip));
    }

    Some(json!({
        "tag": HOSTS_SERVER_TAG,
        "type": "hosts",
        "predefined": predefined,
    }))
}

fn hosts_rule(settings: &AppSettings) -> Value {
    let domains: Vec<&str> = settings
        .dns
        .hosts
        .iter()
        .map(|h| h.domain.as_str())
        .collect();
    json!({ "domain": domains, "server": HOSTS_SERVER_TAG })
}

/// Whether a `hosts` server will exist in the generated config and answer for
/// `host`. Dial-time resolution does not consult `dns.rules`, so an outbound
/// only reaches the pin by naming the server directly.
fn pinned_by_hosts(settings: &AppSettings, host: &str) -> bool {
    (settings.dns.enabled || settings.tun.enabled)
        && host.parse::<std::net::IpAddr>().is_err()
        && settings.dns.hosts.iter().any(|h| h.domain == host)
}

fn strategy_str(strategy: DnsStrategy) -> &'static str {
    match strategy {
        DnsStrategy::PreferIpv4 => "prefer_ipv4",
        DnsStrategy::PreferIpv6 => "prefer_ipv6",
        DnsStrategy::Ipv4Only => "ipv4_only",
        DnsStrategy::Ipv6Only => "ipv6_only",
    }
}

const BOOTSTRAP_RESOLVER_TAG: &str = "sys-dns-bootstrap";

fn default_domain_resolver_tag(settings: &AppSettings) -> Option<String> {
    settings
        .dns
        .servers
        .iter()
        .find(|s| s.address.parse::<std::net::IpAddr>().is_ok())
        .map(|s| s.tag.clone())
        .or_else(|| (!settings.dns.servers.is_empty()).then(|| BOOTSTRAP_RESOLVER_TAG.to_string()))
}

fn build_inbounds(settings: &AppSettings) -> Value {
    let mut inbounds = vec![
        json!({
            "type": "mixed",
            "tag": "mixed-in",
            "listen": settings.listen_address,
            "listen_port": settings.socks_port,
        }),
        json!({
            "type": "http",
            "tag": "http-in",
            "listen": settings.listen_address,
            "listen_port": settings.http_port,
        }),
    ];

    if settings.tun.enabled {
        inbounds.push(build_tun_inbound(&settings.tun));
    }

    Value::Array(inbounds)
}

fn build_tun_inbound(tun: &TunConfig) -> Value {
    let mut inbound = json!({
        "type": "tun",
        "tag": "tun-in",
        "interface_name": tun.interface_name,
        "address": tun.addresses(),
        "mtu": tun.mtu,
        "auto_route": true,
        "strict_route": tun.strict_route,
        "stack": tun.stack,
    });

    if !tun.exclude_routes.is_empty() {
        inbound["route_exclude_address"] = json!(tun.exclude_routes);
    }

    inbound
}

fn build_outbounds(nodes: &[ProxyNode], settings: &AppSettings) -> Result<Value, ConfigError> {
    let mut outbounds: Vec<Value> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| -> Result<Value, ConfigError> {
            let tag = super::common::outbound_tag(node, i);
            let mut out = build_outbound(node, &tag)?;
            // Naming the pin as this outbound's own resolver is what keeps the
            // proxy hostname off the resolver that is only reachable through
            // the proxy.
            if pinned_by_hosts(settings, node.address()) {
                out["domain_resolver"] = json!(HOSTS_SERVER_TAG);
            }
            Ok(out)
        })
        .collect::<Result<_, _>>()?;

    outbounds.push(json!({
        "type": "direct",
        "tag": "direct",
    }));
    outbounds.push(json!({
        "type": "block",
        "tag": "block",
    }));

    Ok(Value::Array(outbounds))
}

fn build_outbound(node: &ProxyNode, tag: &str) -> Result<Value, ConfigError> {
    match node {
        ProxyNode::Vless(c) => build_vless(c, tag),
        ProxyNode::Vmess(c) => build_vmess(c, tag),
        ProxyNode::Shadowsocks(c) => Ok(build_ss(c, tag)),
        ProxyNode::Trojan(c) => build_trojan(c, tag),
    }
}

/// Builds a single sing-box outbound for the given node and tag. Shared with
/// the Real Delay probe config generator so probes dial through the exact same
/// outbound shape as a normal run.
pub(crate) fn build_singbox_outbound(node: &ProxyNode, tag: &str) -> Result<Value, ConfigError> {
    build_outbound(node, tag)
}

fn build_vless(c: &VlessConfig, tag: &str) -> Result<Value, ConfigError> {
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

    apply_transport(&mut out, c.remark.as_deref(), &c.address, &c.transport)?;
    apply_tls(&mut out, c.tls.as_ref());
    Ok(out)
}

fn build_vmess(c: &VmessConfig, tag: &str) -> Result<Value, ConfigError> {
    let mut out = json!({
        "type": "vmess",
        "tag": tag,
        "server": c.address,
        "server_port": c.port,
        "uuid": c.uuid,
        "alter_id": c.alter_id,
        "security": c.security,
    });

    apply_transport(&mut out, c.remark.as_deref(), &c.address, &c.transport)?;
    apply_tls(&mut out, c.tls.as_ref());
    Ok(out)
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

fn build_trojan(c: &TrojanConfig, tag: &str) -> Result<Value, ConfigError> {
    let mut out = json!({
        "type": "trojan",
        "tag": tag,
        "server": c.address,
        "server_port": c.port,
        "password": c.password,
    });

    apply_transport(&mut out, c.remark.as_deref(), &c.address, &c.transport)?;
    apply_tls(&mut out, c.tls.as_ref());
    Ok(out)
}

fn apply_transport(
    out: &mut Value,
    remark: Option<&str>,
    address: &str,
    transport: &TransportSettings,
) -> Result<(), ConfigError> {
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
        TransportSettings::Xhttp(_) => {
            return Err(ConfigError::UnsupportedTransport {
                backend: BackendType::SingBox,
                node: remark.unwrap_or(address).to_string(),
            });
        }
    }
    Ok(())
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

fn build_dns(rules: &[RoutingRule], settings: &AppSettings, first_proxy_tag: &str) -> Value {
    let mut dns_config = json!({});

    dns_config["strategy"] = json!(strategy_str(settings.dns.strategy));

    let mut servers: Vec<Value> = Vec::new();

    if let Some(hosts) = hosts_server(settings) {
        servers.push(hosts);
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
                "path": "/dns-query",
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

        // Only a proxy detour is expressible. `detour: "direct"` names the
        // empty direct outbound, which sing-box refuses to start against
        // ("detour to an empty direct outbound makes no sense") while
        // `sing-box check` accepts it; a server with no detour is not
        // dispatched through the proxy chain in the first place.
        if server_cfg.detour.is_some() && !server_cfg.detours_direct() {
            server["detour"] = json!(first_proxy_tag);
        }

        // sing-box rejects a DNS server addressed by hostname at startup
        // ("missing domain resolver for domain server address") unless that
        // server carries its own `domain_resolver` dial field - the route-level
        // default_domain_resolver does not cover DNS server initialization
        // itself. `default_domain_resolver_tag` never returns this server's own
        // tag (it only picks an IP-literal server or the bootstrap), so no
        // self-reference is possible here.
        if server_cfg.address.parse::<std::net::IpAddr>().is_err()
            && let Some(resolver) = default_domain_resolver_tag(settings)
        {
            server["domain_resolver"] = json!(resolver);
        }

        if let Some(ref client_subnet) = settings.dns.client_subnet {
            server["client_subnet"] = json!(client_subnet);
        }

        servers.push(server);
    }

    if settings.dns.fakeip.enabled {
        servers.push(json!({
            "tag": "fakeip",
            "type": "fakeip",
            "inet4_range": settings.dns.fakeip.inet4_range,
            "inet6_range": settings.dns.fakeip.inet6_range,
        }));
    }

    // A DNS server addressed by hostname (common for public DoH, e.g. dns.google)
    // can't resolve itself. When none of the configured servers is IP-literal,
    // `default_domain_resolver_tag` falls back to this local-system bootstrap
    // so sing-box has somewhere non-circular to resolve those hostnames.
    if !settings.dns.servers.is_empty()
        && !settings
            .dns
            .servers
            .iter()
            .any(|s| s.address.parse::<std::net::IpAddr>().is_ok())
    {
        servers.push(json!({
            "tag": BOOTSTRAP_RESOLVER_TAG,
            "type": "local",
        }));
    }

    let final_server = settings.dns.servers.first().map(|s| s.tag.clone());
    dns_config["final"] = json!(final_server);

    dns_config["servers"] = json!(servers);

    let mut tun_exclusion_rules: Vec<Value> = Vec::new();
    if settings.tun.enabled
        && let Some(direct_tag) =
            super::common::split_horizon_server(settings).map(|s| s.tag.clone())
    {
        if !settings.tun.exclude_processes.is_empty() {
            tun_exclusion_rules.push(json!({
                "process_name": &settings.tun.exclude_processes,
                "server": &direct_tag,
            }));
        }
        if !settings.tun.exclude_domains.is_empty() {
            tun_exclusion_rules.push(json!({
                "domain_suffix": &settings.tun.exclude_domains,
                "server": &direct_tag,
            }));
        }
    }

    let mut dns_rules: Vec<Value> = if settings.dns.use_custom_rules {
        let custom: Vec<Value> = settings
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
                DnsRuleMatch::DomainKeyword { keyword } => json!({
                    "domain_keyword": [keyword],
                    "server": rule.server_tag,
                }),
                DnsRuleMatch::DomainFull { domain } => json!({
                    "domain": [domain],
                    "server": rule.server_tag,
                }),
            })
            .collect();
        let mut all = tun_exclusion_rules;
        all.extend(custom);
        all
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

        let mut all = tun_exclusion_rules;
        all.extend(derived_rules);
        all
    };

    if !settings.dns.hosts.is_empty() {
        dns_rules.insert(0, hosts_rule(settings));
    }

    if settings.dns.fakeip.enabled {
        dns_rules.push(json!({ "query_type": ["A", "AAAA"], "server": "fakeip" }));
    }

    dns_config["rules"] = json!(dns_rules);

    if settings.dns.disable_cache {
        dns_config["disable_cache"] = json!(true);
    }

    dns_config
}

fn build_route(
    rules: &[RoutingRule],
    first_proxy_tag: &str,
    via_tags: &[(ConnectionNodeRef, String)],
    settings: &AppSettings,
) -> Value {
    let enabled: Vec<&RoutingRule> = rules.iter().filter(|r| r.enabled).collect();

    let mut route_rules: Vec<Value> = Vec::new();

    let needs_protocol_sniff = enabled
        .iter()
        .any(|r| matches!(r.match_condition, RuleMatch::Protocol { .. }));
    if needs_protocol_sniff {
        // `protocol` rules match the sniffed protocol; mixed-in/http-in never
        // sniff on their own (tun-in gets its own sniff rule below).
        route_rules.push(json!({
            "inbound": ["mixed-in", "http-in"],
            "action": "sniff",
        }));
    }

    if settings.tun.enabled {
        route_rules.push(json!({
            "inbound": ["tun-in"],
            "action": "sniff",
        }));
        if settings.tun.dns_hijack == DnsHijackMode::Hijack {
            route_rules.push(json!({
                "protocol": "dns",
                "action": "hijack-dns",
            }));
        }

        if !settings.tun.exclude_processes.is_empty() {
            route_rules.push(json!({
                "process_name": &settings.tun.exclude_processes,
                "outbound": "direct",
            }));
        }
        if !settings.tun.exclude_domains.is_empty() {
            route_rules.push(json!({
                "domain_suffix": &settings.tun.exclude_domains,
                "outbound": "direct",
            }));
        }
    }

    if enabled.is_empty() && route_rules.is_empty() {
        return json!({ "rules": [] });
    }

    let mut geoip_tags = BTreeSet::new();
    let mut geosite_tags = BTreeSet::new();

    for rule in &enabled {
        match &rule.match_condition {
            // "private" has no downloadable rule-set - build_route_rule emits
            // ip_is_private instead, so it must not be collected here.
            RuleMatch::GeoIp { country_code } if country_code.eq_ignore_ascii_case("private") => {}
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

    // No download_detour: sing-box then downloads through the default outbound
    // (the proxy), the only path that works where GitHub is blocked. The field
    // is also deprecated in sing-box 1.14 and removed in 1.16.
    for tag in &geoip_tags {
        rule_sets.push(json!({
            "type": "remote",
            "tag": format!("geoip-{tag}"),
            "format": "binary",
            "url": format!("{GEOIP_RULESET_URL}/geoip-{tag}.srs"),
        }));
    }
    for tag in &geosite_tags {
        rule_sets.push(json!({
            "type": "remote",
            "tag": format!("geosite-{tag}"),
            "format": "binary",
            "url": format!("{GEOSITE_RULESET_URL}/geosite-{tag}.srs"),
        }));
    }

    let user_rules: Vec<Value> = enabled
        .iter()
        .map(|r| build_route_rule(r, first_proxy_tag, via_tags))
        .collect();
    route_rules.extend(user_rules);

    if rule_sets.is_empty() {
        json!({ "rules": route_rules })
    } else {
        json!({
            "rule_set": rule_sets,
            "rules": route_rules,
        })
    }
}

/// Rewrites `route.rule_set` entries from `remote` to `local` wherever a
/// cached `.srs` file exists, so a run with fully cached geodata never needs
/// network access to start. `rule_sets_dir` must be absolute — sing-box
/// resolves a bare path against its own working directory.
pub(crate) fn apply_local_rule_sets(config: &mut Value, rule_sets_dir: &std::path::Path) {
    let Some(rule_sets) = config["route"]["rule_set"].as_array_mut() else {
        return;
    };

    for entry in rule_sets.iter_mut() {
        if entry["type"] != "remote" {
            continue;
        }
        let Some(tag) = entry["tag"].as_str().map(str::to_string) else {
            continue;
        };
        let local_path = rule_sets_dir.join(format!("{tag}.srs"));
        if !local_path.exists() {
            continue;
        }
        *entry = json!({
            "type": "local",
            "tag": tag,
            "format": "binary",
            "path": local_path,
        });
    }
}

fn build_route_rule(
    rule: &RoutingRule,
    first_proxy_tag: &str,
    via_tags: &[(ConnectionNodeRef, String)],
) -> Value {
    let proxy_tag;
    let outbound = match rule.action {
        RuleAction::Proxy => {
            proxy_tag = super::v2ray::proxy_tag_for(rule, first_proxy_tag, via_tags);
            proxy_tag.as_str()
        }
        RuleAction::Direct => "direct",
        RuleAction::Block => "block",
    };

    match &rule.match_condition {
        // v2fly/xray bundle RFC 1918 private ranges as a "private" GeoIP
        // category; SagerNet's sing-geoip mirror ships no such .srs (404 -
        // there's nothing to download). sing-box has this as a dedicated
        // rule field instead.
        RuleMatch::GeoIp { country_code } if country_code.eq_ignore_ascii_case("private") => {
            json!({
                "ip_is_private": true,
                "outbound": outbound,
            })
        }
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
        RuleMatch::DomainKeyword { keyword } => json!({
            "domain_keyword": [keyword],
            "outbound": outbound,
        }),
        RuleMatch::DomainFull { domain } => json!({
            "domain": [domain],
            "outbound": outbound,
        }),
        RuleMatch::IpCidr { cidr } => json!({
            "ip_cidr": [cidr.to_string()],
            "outbound": outbound,
        }),
        RuleMatch::Protocol { name } => json!({
            "protocol": name,
            "outbound": outbound,
        }),
        RuleMatch::Port { spec } => {
            let mut obj = json!({ "outbound": outbound });
            let (ports, ranges) = split_port_spec(spec);
            if !ports.is_empty() {
                obj["port"] = json!(ports);
            }
            if !ranges.is_empty() {
                obj["port_range"] = json!(ranges);
            }
            obj
        }
        RuleMatch::Network { spec } => json!({
            "network": spec.split(',').collect::<Vec<_>>(),
            "outbound": outbound,
        }),
    }
}

/// Splits a `Port` match spec ("53", "1000-2000", "80,443") into sing-box's
/// separate `port` (single values) and `port_range` (colon-delimited strings)
/// fields.
fn split_port_spec(spec: &str) -> (Vec<u16>, Vec<String>) {
    let mut ports = Vec::new();
    let mut ranges = Vec::new();
    for part in spec.split(',') {
        match part.split_once('-') {
            Some((start, end)) => ranges.push(format!("{start}:{end}")),
            None => {
                if let Ok(port) = part.parse::<u16>() {
                    ports.push(port);
                }
            }
        }
    }
    (ports, ranges)
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

    fn tun_inbound(config: &Value) -> Option<&Value> {
        config["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["type"] == "tun")
    }

    #[test]
    fn test_singbox_tun_inbound_emitted_when_enabled() {
        let mut settings = default_settings();
        settings.tun.enabled = true;

        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &settings)
            .unwrap();

        let tun = tun_inbound(&config).expect("tun inbound missing");
        assert_eq!(tun["auto_route"], true);
        assert_eq!(tun["address"], json!(["172.19.0.1/30"]));
        assert_eq!(tun["mtu"], 1500);
        assert_eq!(tun["stack"], "system");
        assert_eq!(tun["strict_route"], true);
        assert_eq!(tun["interface_name"], "tun0");
        assert!(
            tun.get("dns_mode").is_none(),
            "dns_mode removed in sing-box 1.13+"
        );
        assert!(
            tun.get("sniff").is_none(),
            "legacy sniff field removed in sing-box 1.13.0"
        );

        assert_eq!(config["route"]["auto_detect_interface"], true);
        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert_eq!(route_rules[0]["inbound"], json!(["tun-in"]));
        assert_eq!(route_rules[0]["action"], "sniff");
    }

    #[test]
    fn test_singbox_tun_hijack_dns_rule_when_dns_enabled_and_hijack_mode() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;

        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &settings)
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert_eq!(route_rules[0]["action"], "sniff");
        assert_eq!(route_rules[1]["protocol"], "dns");
        assert_eq!(route_rules[1]["action"], "hijack-dns");
    }

    #[test]
    fn test_singbox_tun_profile_dns_overrides_disabled_global_dns() {
        let mut sub = Subscription::new_from_url("Provider", "https://example.com/sub");
        sub.nodes = vec![SubscriptionNode::new(vless_node())];
        sub.use_imported_profile = true;
        sub.imported_profile = Some(ImportedProfile {
            rules: vec![],
            dns: Some(DnsConfig {
                enabled: true,
                servers: vec![DnsServerConfig {
                    tag: "provider-dns".into(),
                    protocol: DnsProtocol::Doh,
                    address: "1.1.1.1".into(),
                    port: None,
                    detour: None,
                }],
                ..DnsConfig::default()
            }),
            skipped: vec![],
            imported_at: chrono::Utc::now(),
        });
        let node_ref = ConnectionNodeRef::Subscription {
            subscription_id: sub.id,
            node_id: sub.nodes[0].id,
        };

        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = false;

        let (rules, effective) = resolve_effective_config(&node_ref, &[sub], &[], &settings);
        assert!(
            effective.dns.enabled,
            "provider dns must override a disabled global dns"
        );

        let config = SingboxGenerator
            .generate(&[vless_node()], &rules, &effective)
            .unwrap();

        assert_eq!(config["route"]["default_domain_resolver"], "provider-dns");
        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert!(route_rules.iter().any(|r| r["action"] == "hijack-dns"));
    }

    #[test]
    fn test_singbox_tun_with_dns_disabled_derives_dns_plane() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = false;

        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &settings)
            .unwrap();

        let servers = config["dns"]["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["type"], "https");
        assert_eq!(servers[0]["server"], "1.1.1.1");
        assert_eq!(servers[0]["detour"], "proxy-0-Test SS");
        assert_eq!(config["dns"]["final"], servers[0]["tag"]);
        assert_eq!(
            config["route"]["default_domain_resolver"],
            servers[0]["tag"]
        );

        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert!(route_rules.iter().any(|r| r["action"] == "hijack-dns"));
    }

    #[test]
    fn test_singbox_no_derived_dns_without_tun() {
        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &default_settings())
            .unwrap();

        assert!(config.get("dns").is_none());
        assert!(config["route"].get("default_domain_resolver").is_none());
    }

    #[test]
    fn test_singbox_no_hijack_dns_rule_when_hijack_mode_native() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.tun.dns_hijack = DnsHijackMode::Native;

        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &settings)
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert!(!route_rules.iter().any(|r| r["action"] == "hijack-dns"));
    }

    #[test]
    fn test_singbox_tun_includes_ipv6_address() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.address_v6 = Some("fd00::1/126".to_string());

        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &settings)
            .unwrap();

        let tun = tun_inbound(&config).unwrap();
        assert_eq!(tun["address"], json!(["172.19.0.1/30", "fd00::1/126"]));
    }

    #[test]
    fn test_singbox_tun_excluded_routes_mapped() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.exclude_routes = vec!["192.168.0.0/16".to_string()];

        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &settings)
            .unwrap();

        let tun = tun_inbound(&config).unwrap();
        assert_eq!(tun["route_exclude_address"], json!(["192.168.0.0/16"]));
    }

    #[test]
    fn test_singbox_no_tun_inbound_when_disabled() {
        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &default_settings())
            .unwrap();

        assert!(tun_inbound(&config).is_none());
        assert!(config["route"].get("auto_detect_interface").is_none());
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
    fn test_apply_local_rule_sets_cached_tag_becomes_local() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("geoip-ru.srs"), b"fake").unwrap();

        let mut config = json!({
            "route": {
                "rule_set": [
                    {"type": "remote", "tag": "geoip-ru", "format": "binary", "url": "https://example.com/geoip-ru.srs"},
                ],
            },
        });

        apply_local_rule_sets(&mut config, tmp.path());

        let entry = &config["route"]["rule_set"][0];
        assert_eq!(entry["type"], "local");
        assert_eq!(entry["format"], "binary");
        assert_eq!(entry["tag"], "geoip-ru");
        assert!(entry.get("url").is_none());
        let path = entry["path"].as_str().unwrap();
        assert!(std::path::Path::new(path).is_absolute());
        assert!(path.ends_with("geoip-ru.srs"));
    }

    #[test]
    fn test_apply_local_rule_sets_uncached_tag_stays_remote() {
        let tmp = tempfile::TempDir::new().unwrap();

        let mut config = json!({
            "route": {
                "rule_set": [
                    {"type": "remote", "tag": "geoip-ru", "format": "binary", "url": "https://example.com/geoip-ru.srs"},
                ],
            },
        });

        apply_local_rule_sets(&mut config, tmp.path());

        let entry = &config["route"]["rule_set"][0];
        assert_eq!(entry["type"], "remote");
        assert_eq!(entry["url"], "https://example.com/geoip-ru.srs");
    }

    #[test]
    fn test_apply_local_rule_sets_mixed_set() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("geoip-ru.srs"), b"fake").unwrap();

        let mut config = json!({
            "route": {
                "rule_set": [
                    {"type": "remote", "tag": "geoip-ru", "format": "binary", "url": "https://example.com/geoip-ru.srs"},
                    {"type": "remote", "tag": "geosite-google", "format": "binary", "url": "https://example.com/geosite-google.srs"},
                ],
            },
        });

        apply_local_rule_sets(&mut config, tmp.path());

        let rule_sets = config["route"]["rule_set"].as_array().unwrap();
        assert_eq!(rule_sets[0]["type"], "local");
        assert_eq!(rule_sets[1]["type"], "remote");
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
            via_node: None,
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
        assert!(
            rule_sets[0].get("download_detour").is_none(),
            "download_detour must be absent so rule-sets download via the default (proxy) outbound"
        );
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
            via_node: None,
        }];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings())
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert_eq!(route_rules[0]["rule_set"][0], "geosite-google");
    }

    #[test]
    fn test_singbox_xhttp_unsupported_transport() {
        let generator = SingboxGenerator;
        let result = generator.generate(&[xhttp_node()], &[], &default_settings());

        match result {
            Err(ConfigError::UnsupportedTransport { backend, node }) => {
                assert_eq!(backend, BackendType::SingBox);
                assert_eq!(node, "Test XHTTP");
            }
            other => panic!("expected UnsupportedTransport, got {other:?}"),
        }
    }

    #[test]
    fn test_singbox_domain_route_stays_suffix() {
        let generator = SingboxGenerator;
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Domain {
                pattern: "example.com".into(),
            },
            action: RuleAction::Proxy,
            enabled: true,
            group: None,
            via_node: None,
        }];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings())
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert_eq!(route_rules[0]["domain_suffix"][0], "example.com");
    }

    #[test]
    fn test_singbox_domain_keyword_route() {
        let generator = SingboxGenerator;
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::DomainKeyword {
                keyword: "sina".into(),
            },
            action: RuleAction::Proxy,
            enabled: true,
            group: None,
            via_node: None,
        }];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings())
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert_eq!(route_rules[0]["domain_keyword"][0], "sina");
    }

    #[test]
    fn test_singbox_domain_full_route() {
        let generator = SingboxGenerator;
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::DomainFull {
                domain: "example.com".into(),
            },
            action: RuleAction::Proxy,
            enabled: true,
            group: None,
            via_node: None,
        }];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings())
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        assert_eq!(route_rules[0]["domain"][0], "example.com");
    }

    #[test]
    fn test_singbox_protocol_route_and_sniff() {
        let generator = SingboxGenerator;
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Protocol {
                name: "bittorrent".into(),
            },
            action: RuleAction::Block,
            enabled: true,
            group: None,
            via_node: None,
        }];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings())
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        let sniff_rule = route_rules
            .iter()
            .find(|r| r["action"] == "sniff")
            .expect("sniff rule missing for protocol match");
        assert_eq!(sniff_rule["inbound"], json!(["mixed-in", "http-in"]));

        let protocol_rule = route_rules
            .iter()
            .find(|r| r.get("protocol").is_some())
            .unwrap();
        assert_eq!(protocol_rule["protocol"], "bittorrent");
        assert_eq!(protocol_rule["outbound"], "block");
    }

    #[test]
    fn test_singbox_port_route() {
        let generator = SingboxGenerator;
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Port {
                spec: "80,443,1000-2000".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
            group: None,
            via_node: None,
        }];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings())
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        let port_rule = route_rules
            .iter()
            .find(|r| r.get("port").is_some() || r.get("port_range").is_some())
            .unwrap();
        assert_eq!(port_rule["port"], json!([80, 443]));
        assert_eq!(port_rule["port_range"], json!(["1000:2000"]));
    }

    #[test]
    fn test_singbox_network_route() {
        let generator = SingboxGenerator;
        let rules = vec![RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: RuleMatch::Network {
                spec: "tcp,udp".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
            group: None,
            via_node: None,
        }];

        let config = generator
            .generate(&[ss_node()], &rules, &default_settings())
            .unwrap();

        let route_rules = config["route"]["rules"].as_array().unwrap();
        let network_rule = route_rules
            .iter()
            .find(|r| r.get("network").is_some())
            .unwrap();
        assert_eq!(network_rule["network"], json!(["tcp", "udp"]));
    }

    #[test]
    fn test_singbox_dns_custom_rules_domain_keyword_and_full() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.use_custom_rules = true;
        settings.dns.rules = vec![
            DnsRule {
                match_condition: DnsRuleMatch::DomainKeyword {
                    keyword: "sina".to_string(),
                },
                server_tag: "remote".to_string(),
            },
            DnsRule {
                match_condition: DnsRuleMatch::DomainFull {
                    domain: "example.com".to_string(),
                },
                server_tag: "domestic".to_string(),
            },
        ];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let rules = config["dns"]["rules"].as_array().unwrap();
        let keyword_rule = rules
            .iter()
            .find(|r| r.get("domain_keyword").is_some())
            .unwrap();
        assert_eq!(keyword_rule["domain_keyword"], json!(["sina"]));

        let full_rule = rules.iter().find(|r| r["server"] == "domestic").unwrap();
        assert_eq!(full_rule["domain"], json!(["example.com"]));
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
                via_node: None,
            },
            RoutingRule {
                id: uuid::Uuid::new_v4(),
                match_condition: RuleMatch::GeoSite {
                    category: "google".into(),
                },
                action: RuleAction::Proxy,
                enabled: true,
                group: None,
                via_node: None,
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
    fn test_dns_default_domain_resolver_picks_ip_literal_server() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![
            DnsServerConfig {
                tag: "domain-server".to_string(),
                protocol: DnsProtocol::Doh,
                address: "dns.google".to_string(),
                port: None,
                detour: None,
            },
            DnsServerConfig {
                tag: "ip-server".to_string(),
                protocol: DnsProtocol::Udp,
                address: "8.8.8.8".to_string(),
                port: None,
                detour: None,
            },
        ];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        assert_eq!(config["route"]["default_domain_resolver"], "ip-server");
    }

    #[test]
    fn test_dns_default_domain_resolver_falls_back_to_bootstrap_when_all_servers_are_hostnames() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "domain-server".to_string(),
            protocol: DnsProtocol::Doh,
            address: "dns.google".to_string(),
            port: None,
            detour: None,
        }];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        // "domain-server" can't resolve its own hostname (circular) - sing-box
        // rejects that config at startup with "missing domain resolver for
        // domain server address". Must fall back to a local bootstrap instead.
        assert_eq!(
            config["route"]["default_domain_resolver"],
            BOOTSTRAP_RESOLVER_TAG
        );
        let servers = config["dns"]["servers"].as_array().unwrap();
        let bootstrap = servers
            .iter()
            .find(|s| s["tag"] == BOOTSTRAP_RESOLVER_TAG)
            .expect("bootstrap resolver server must be present in dns.servers");
        assert_eq!(bootstrap["type"], "local");
    }

    #[test]
    fn test_dns_no_bootstrap_injected_when_ip_literal_server_present() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "ip-server".to_string(),
            protocol: DnsProtocol::Doh,
            address: "1.1.1.1".to_string(),
            port: None,
            detour: None,
        }];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        assert_eq!(config["route"]["default_domain_resolver"], "ip-server");
        let servers = config["dns"]["servers"].as_array().unwrap();
        assert!(
            !servers.iter().any(|s| s["tag"] == BOOTSTRAP_RESOLVER_TAG),
            "no bootstrap should be injected when a resolvable server already exists"
        );
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
        assert_eq!(doh_server["path"], "/dns-query");
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

        assert!(
            config["dns"].get("fakeip").is_none(),
            "legacy top-level dns.fakeip block deprecated in sing-box 1.12.0"
        );

        let servers = config["dns"]["servers"].as_array().unwrap();
        let fakeip_server = servers.iter().find(|s| s["tag"] == "fakeip").unwrap();
        assert_eq!(fakeip_server["type"], "fakeip");
        assert_eq!(fakeip_server["inet4_range"], "198.18.0.0/16");
        assert_eq!(fakeip_server["inet6_range"], "fc00::/16");

        assert_ne!(
            config["dns"]["final"], "fakeip",
            "a fakeip server cannot be the default resolver"
        );
        assert_eq!(config["dns"]["final"], settings.dns.servers[0].tag);

        let dns_rules = config["dns"]["rules"].as_array().unwrap();
        let fakeip_rule = dns_rules
            .iter()
            .find(|r| r["server"] == "fakeip")
            .expect("fakeip query_type rule not found");
        assert_eq!(fakeip_rule["query_type"], json!(["A", "AAAA"]));
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
        assert_eq!(
            hosts_server["predefined"]["example.com"],
            json!(["192.0.2.1"])
        );
        assert_eq!(
            hosts_server["predefined"]["test.local"],
            json!(["192.0.2.2"])
        );

        let dns_rules = config["dns"]["rules"].as_array().unwrap();
        assert_eq!(dns_rules[0]["server"], "hosts");
        assert_eq!(dns_rules[0]["domain"], json!(["example.com", "test.local"]));
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
    fn test_dns_server_detour_resolves_legacy_sentinel_to_real_outbound_tag() {
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
        assert_eq!(proxy_dns["detour"], "proxy-0-Test SS");
    }

    /// `detour: "direct"` names the empty direct outbound, which sing-box
    /// refuses to start against even though `sing-box check` accepts it. The
    /// server has to carry no detour at all.
    #[test]
    fn test_dns_server_direct_detour_is_not_emitted() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "direct-dns".to_string(),
            protocol: DnsProtocol::Udp,
            address: "223.5.5.5".to_string(),
            port: None,
            detour: Some("direct".to_string()),
        }];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let servers = config["dns"]["servers"].as_array().unwrap();
        let direct_dns = servers.iter().find(|s| s["tag"] == "direct-dns").unwrap();
        assert!(
            direct_dns.get("detour").is_none(),
            "a direct detour must not reach the config: {direct_dns}"
        );
    }

    #[test]
    fn test_derived_tun_dns_carries_the_host_pin() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = false;
        settings.dns.hosts = vec![HostOverride {
            domain: "ss.example.com".into(),
            ip: "203.0.113.9".into(),
        }];

        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &settings)
            .unwrap();

        let hosts = config["dns"]["servers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["tag"] == "hosts")
            .expect("the derived plane dropped the pin");
        assert_eq!(
            hosts["predefined"]["ss.example.com"],
            json!(["203.0.113.9"])
        );
        assert_eq!(config["dns"]["rules"][0]["server"], "hosts");
    }

    /// Dial-time resolution does not consult `dns.rules`, so the pin only
    /// reaches the proxy's own hostname when the outbound names it.
    #[test]
    fn test_pinned_node_resolves_through_the_hosts_server() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.hosts = vec![HostOverride {
            domain: "ss.example.com".into(),
            ip: "203.0.113.9".into(),
        }];

        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &settings)
            .unwrap();

        let proxy = &config["outbounds"][0];
        assert_eq!(proxy["domain_resolver"], "hosts");
        assert_ne!(
            config["route"]["default_domain_resolver"], "hosts",
            "the general resolver must not inherit the pin's NXDOMAIN on a miss"
        );
    }

    #[test]
    fn test_unpinned_node_gets_no_hosts_resolver() {
        let mut settings = default_settings();
        settings.tun.enabled = true;

        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &settings)
            .unwrap();

        assert!(config["outbounds"][0].get("domain_resolver").is_none());
    }

    /// Without a TUN or DNS section there is no `hosts` server to point at.
    #[test]
    fn test_hosts_resolver_is_not_named_when_no_dns_section_exists() {
        let mut settings = default_settings();
        settings.dns.enabled = false;
        settings.tun.enabled = false;
        settings.dns.hosts = vec![HostOverride {
            domain: "ss.example.com".into(),
            ip: "203.0.113.9".into(),
        }];

        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &settings)
            .unwrap();

        assert!(config.get("dns").is_none());
        assert!(config["outbounds"][0].get("domain_resolver").is_none());
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
                via_node: None,
            },
            RoutingRule {
                id: uuid::Uuid::new_v4(),
                match_condition: RuleMatch::GeoSite {
                    category: "cn".to_string(),
                },
                action: RuleAction::Direct,
                enabled: true,
                group: None,
                via_node: None,
            },
            RoutingRule {
                id: uuid::Uuid::new_v4(),
                match_condition: RuleMatch::Domain {
                    pattern: ".example.com".to_string(),
                },
                action: RuleAction::Proxy,
                enabled: true,
                group: None,
                via_node: None,
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
        assert_eq!(
            hosts_server["predefined"]["example.com"],
            json!(["192.0.2.1"])
        );
        assert_eq!(
            hosts_server["predefined"]["test.local"],
            json!(["10.0.0.1"])
        );

        let fakeip_server = servers
            .iter()
            .find(|s| s["tag"] == "fakeip")
            .expect("fakeip server not found");
        assert_eq!(fakeip_server["type"], "fakeip");
        assert_eq!(fakeip_server["inet4_range"], "198.18.0.0/16");
        assert_eq!(fakeip_server["inet6_range"], "fc00::/16");
        assert!(dns.get("fakeip").is_none());
        assert_eq!(dns["final"], "cloudflare");

        let cloudflare = servers
            .iter()
            .find(|s| s["tag"] == "cloudflare")
            .expect("cloudflare server not found");
        assert_eq!(cloudflare["client_subnet"], "203.0.113.1");
        assert_eq!(cloudflare["detour"], "proxy-0-Test SS");

        let google = servers
            .iter()
            .find(|s| s["tag"] == "google")
            .expect("google server not found");
        assert_eq!(google["client_subnet"], "203.0.113.1");

        let dns_rules = dns["rules"].as_array().unwrap();
        assert_eq!(dns_rules.len(), 4);
        assert_eq!(dns_rules[0]["server"], "hosts");
        assert_eq!(dns_rules[3]["server"], "fakeip");
        assert_eq!(dns_rules[3]["query_type"], json!(["A", "AAAA"]));

        assert_eq!(config["route"]["default_domain_resolver"], "cloudflare");

        let json_str = serde_json::to_string(&config).unwrap();
        let _: Value = serde_json::from_str(&json_str).unwrap();
    }

    #[test]
    fn test_singbox_tun_exclusion_process_name() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.exclude_processes = vec!["cloudflared".to_string()];

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let rules = config["route"]["rules"].as_array().unwrap();
        let process_rule = rules
            .iter()
            .find(|r| r.get("process_name").is_some())
            .expect("process_name route rule not found");
        assert_eq!(process_rule["process_name"], json!(["cloudflared"]));
        assert_eq!(process_rule["outbound"], "direct");
    }

    #[test]
    fn test_singbox_tun_exclusion_domain() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.exclude_domains = vec!["example.com".to_string()];
        settings.dns.enabled = true;

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let rules = config["route"]["rules"].as_array().unwrap();
        let domain_rule = rules
            .iter()
            .find(|r| r.get("domain_suffix").is_some())
            .expect("domain_suffix route rule not found");
        assert_eq!(domain_rule["domain_suffix"], json!(["example.com"]));
        assert_eq!(domain_rule["outbound"], "direct");

        let dns_rules = config["dns"]["rules"].as_array().unwrap();
        let dns_first = dns_rules
            .iter()
            .find(|r| r.get("domain_suffix").is_some())
            .expect("domain_suffix DNS rule not found");
        assert_eq!(dns_first["domain_suffix"], json!(["example.com"]));
    }

    #[test]
    fn test_singbox_excluded_domains_use_the_direct_detoured_server() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.exclude_domains = vec!["corp.example".to_string()];
        settings.dns.enabled = true;
        settings.dns.servers = vec![
            DnsServerConfig {
                tag: "remote".into(),
                protocol: DnsProtocol::Doh,
                address: "1.1.1.1".into(),
                port: None,
                detour: Some("proxy".into()),
            },
            DnsServerConfig {
                tag: "domestic".into(),
                protocol: DnsProtocol::Udp,
                address: "77.88.8.8".into(),
                port: None,
                detour: Some("direct".into()),
            },
        ];

        let config = SingboxGenerator
            .generate(&[ss_node()], &[], &settings)
            .unwrap();

        let dns_rules = config["dns"]["rules"].as_array().unwrap();
        let rule = dns_rules
            .iter()
            .find(|r| r.get("domain_suffix").is_some())
            .expect("domain_suffix DNS rule not found");
        assert_eq!(rule["server"], "domestic");
    }

    #[test]
    fn test_singbox_tun_exclusion_dns_process() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.exclude_processes = vec!["cloudflared".to_string()];
        settings.dns.enabled = true;

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let dns_rules = config["dns"]["rules"].as_array().unwrap();
        let proc_rule = dns_rules
            .iter()
            .find(|r| r.get("process_name").is_some())
            .expect("process_name DNS rule not found");
        assert_eq!(proc_rule["process_name"], json!(["cloudflared"]));
    }

    #[test]
    fn test_singbox_no_exclusion_when_tun_disabled() {
        let mut settings = default_settings();
        settings.tun.exclude_processes = vec!["cloudflared".to_string()];
        settings.tun.exclude_domains = vec!["example.com".to_string()];
        settings.dns.enabled = true;

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let rules = config["route"]["rules"].as_array().unwrap();
        for rule in rules {
            assert!(rule.get("process_name").is_none());
            assert!(rule.get("domain_suffix").is_none());
        }

        let dns_rules = config["dns"]["rules"].as_array().unwrap();
        for rule in dns_rules {
            assert!(rule.get("process_name").is_none());
        }
    }

    #[test]
    fn test_singbox_no_exclusion_when_lists_empty() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;

        let generator = SingboxGenerator;
        let config = generator.generate(&[ss_node()], &[], &settings).unwrap();

        let rules = config["route"]["rules"].as_array().unwrap();
        for rule in rules {
            assert!(rule.get("process_name").is_none());
            assert!(rule.get("domain_suffix").is_none());
        }
    }
}
