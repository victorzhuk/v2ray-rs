use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;
use v2ray_rs_core::models::{
    DnsConfig, DnsProtocol, DnsRule, DnsRuleMatch, DnsServerConfig, DnsStrategy, GrpcSettings,
    H2Settings, HostOverride, ImportedProfile, ProxyNode, RoutingRule, RuleAction, RuleMatch,
    ShadowsocksConfig, SubscriptionNode, TlsSettings, TransportSettings, TrojanConfig, VlessConfig,
    VmessConfig, WsSettings, XhttpSettings, validate_dns_rule_match, validate_rule_match,
};

use crate::parser::{ImportResult, ParseError};

/// Config outbound protocols that never carry a proxy node.
const NON_PROXY_PROTOCOLS: &[&str] = &["freedom", "blackhole", "dns", "loopback"];

pub struct JsonImport {
    pub result: ImportResult,
    pub profile: Option<ImportedProfile>,
}

/// Parses a v2rayTun/v2rayN/v2rayNG-style "config bundle" subscription body:
/// a JSON array (or single object) of complete backend configs, one per node,
/// sharing routing/DNS. Returns `None` when `raw` isn't this format, so the
/// caller falls back to the base64 URI-list path.
pub fn parse_json_subscription(raw: &str) -> Option<JsonImport> {
    let elements = detect_bundle(raw)?;
    if elements.is_empty() {
        return None;
    }

    let mut nodes = Vec::new();
    let mut errors = Vec::new();
    let mut skipped = Vec::new();
    let mut profile_routing: Option<Value> = None;
    let mut profile_dns: Option<Value> = None;

    for (i, element) in elements.iter().enumerate() {
        let label = element
            .get("remarks")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("entry {i}"));

        match import_node(element) {
            Ok(node) => match node.validate() {
                Ok(()) => nodes.push(SubscriptionNode::new(node)),
                Err(e) => errors.push((label, ParseError::InvalidFormat(e.to_string()))),
            },
            Err(reason) => errors.push((label, ParseError::InvalidFormat(reason))),
        }

        let routing = element.get("routing").cloned();
        let dns = element.get("dns").cloned();
        if i == 0 {
            profile_routing = routing;
            profile_dns = dns;
        } else {
            if routing != profile_routing {
                skipped.push(format!(
                    "entry {i}: routing differs from the shared profile, ignored"
                ));
            }
            if dns != profile_dns {
                skipped.push(format!(
                    "entry {i}: dns differs from the shared profile, ignored"
                ));
            }
        }
    }

    let outbounds: Vec<Value> = elements[0]
        .get("outbounds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut rules = Vec::new();
    if let Some(routing) = &profile_routing {
        let (parsed_rules, rule_skips) = map_routing_rules(routing, &outbounds);
        rules = parsed_rules;
        skipped.extend(rule_skips);
    }

    let mut dns_config = None;
    if let Some(dns) = &profile_dns {
        let (parsed_dns, dns_skips) = map_dns_config(dns);
        dns_config = parsed_dns;
        skipped.extend(dns_skips);
    }

    if let Some(cfg) = &dns_config
        && let Err(e) = cfg.validate()
    {
        skipped.push(format!("dns profile invalid, dropped: {e}"));
        dns_config = None;
    }

    let profile = ImportedProfile {
        rules,
        dns: dns_config,
        skipped,
        imported_at: Utc::now(),
    };

    Some(JsonImport {
        result: ImportResult { nodes, errors },
        profile: Some(profile),
    })
}

fn detect_bundle(raw: &str) -> Option<Vec<Value>> {
    if let Some(elements) = try_parse_bundle(raw) {
        return Some(elements);
    }

    let trimmed = raw.trim();
    let decoded = STANDARD
        .decode(trimmed)
        .or_else(|_| URL_SAFE_NO_PAD.decode(trimmed))
        .ok()?;
    let text = String::from_utf8(decoded).ok()?;
    try_parse_bundle(&text)
}

fn try_parse_bundle(text: &str) -> Option<Vec<Value>> {
    if let Ok(array) = serde_json::from_str::<Vec<Value>>(text) {
        return Some(array);
    }
    if let Ok(value @ Value::Object(_)) = serde_json::from_str::<Value>(text)
        && value.get("outbounds").is_some()
    {
        return Some(vec![value]);
    }
    None
}

fn import_node(element: &Value) -> Result<ProxyNode, String> {
    let outbounds = element
        .get("outbounds")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing outbounds array".to_string())?;

    let outbound = outbounds
        .iter()
        .find(|o| {
            let protocol = o.get("protocol").and_then(Value::as_str).unwrap_or("");
            !NON_PROXY_PROTOCOLS.contains(&protocol)
        })
        .ok_or_else(|| "no proxy outbound found".to_string())?;

    let protocol = outbound
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or("");
    let remark = element
        .get("remarks")
        .and_then(Value::as_str)
        .map(str::to_string);
    let stream = outbound.get("streamSettings");
    let transport = stream.map(map_transport).transpose()?.unwrap_or_default();
    let tls = match stream {
        Some(stream) => map_tls(stream)?,
        None => None,
    };

    match protocol {
        "vless" => build_vless(outbound, remark, transport, tls),
        "vmess" => build_vmess(outbound, remark, transport, tls),
        "trojan" => build_trojan(outbound, remark, transport, tls),
        "shadowsocks" => build_ss(outbound, remark),
        other => Err(format!("unsupported protocol: {other}")),
    }
}

fn as_port(v: &Value) -> Option<u16> {
    v.as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn build_vless(
    outbound: &Value,
    remark: Option<String>,
    transport: TransportSettings,
    tls: Option<TlsSettings>,
) -> Result<ProxyNode, String> {
    let vnext = &outbound["settings"]["vnext"][0];
    let user = &vnext["users"][0];
    let address = vnext["address"]
        .as_str()
        .ok_or("vless: missing address")
        .map(crate::parser::normalize_host)?;
    let port = as_port(&vnext["port"]).ok_or("vless: missing port")?;
    let uuid = user["id"].as_str().unwrap_or_default().to_string();
    let encryption = user["encryption"].as_str().map(str::to_string);
    let flow = user["flow"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    Ok(ProxyNode::Vless(VlessConfig {
        address,
        port,
        uuid,
        encryption,
        flow,
        transport,
        tls,
        remark,
    }))
}

fn build_vmess(
    outbound: &Value,
    remark: Option<String>,
    transport: TransportSettings,
    tls: Option<TlsSettings>,
) -> Result<ProxyNode, String> {
    let vnext = &outbound["settings"]["vnext"][0];
    let user = &vnext["users"][0];
    let address = vnext["address"]
        .as_str()
        .ok_or("vmess: missing address")
        .map(crate::parser::normalize_host)?;
    let port = as_port(&vnext["port"]).ok_or("vmess: missing port")?;
    let uuid = user["id"].as_str().unwrap_or_default().to_string();
    let alter_id = user["alterId"].as_u64().unwrap_or(0) as u32;
    let security = user["security"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("auto")
        .to_string();

    Ok(ProxyNode::Vmess(VmessConfig {
        address,
        port,
        uuid,
        alter_id,
        security,
        transport,
        tls,
        remark,
    }))
}

fn build_trojan(
    outbound: &Value,
    remark: Option<String>,
    transport: TransportSettings,
    tls: Option<TlsSettings>,
) -> Result<ProxyNode, String> {
    let server = &outbound["settings"]["servers"][0];
    let address = server["address"]
        .as_str()
        .ok_or("trojan: missing address")
        .map(crate::parser::normalize_host)?;
    let port = as_port(&server["port"]).ok_or("trojan: missing port")?;
    let password = server["password"].as_str().unwrap_or_default().to_string();

    Ok(ProxyNode::Trojan(TrojanConfig {
        address,
        port,
        password,
        transport,
        tls,
        remark,
    }))
}

fn build_ss(outbound: &Value, remark: Option<String>) -> Result<ProxyNode, String> {
    let server = &outbound["settings"]["servers"][0];
    let address = server["address"]
        .as_str()
        .ok_or("shadowsocks: missing address")
        .map(crate::parser::normalize_host)?;
    let port = as_port(&server["port"]).ok_or("shadowsocks: missing port")?;
    let method = server["method"].as_str().unwrap_or_default().to_string();
    let password = server["password"].as_str().unwrap_or_default().to_string();

    Ok(ProxyNode::Shadowsocks(ShadowsocksConfig {
        address,
        port,
        method,
        password,
        remark,
    }))
}

fn map_transport(stream: &Value) -> Result<TransportSettings, String> {
    let network = stream
        .get("network")
        .and_then(Value::as_str)
        .unwrap_or("tcp");

    match network {
        "tcp" => Ok(TransportSettings::Tcp),
        "ws" => {
            let ws = &stream["wsSettings"];
            let path = ws["path"].as_str().unwrap_or_default().to_string();
            let host = ws["headers"]["Host"]
                .as_str()
                .or_else(|| ws["host"].as_str())
                .map(str::to_string);
            Ok(TransportSettings::Ws(WsSettings {
                path,
                host,
                headers: HashMap::new(),
            }))
        }
        "grpc" => {
            let grpc = &stream["grpcSettings"];
            let service_name = grpc["serviceName"].as_str().unwrap_or_default().to_string();
            let multi_mode = grpc["multiMode"].as_bool().unwrap_or(false);
            Ok(TransportSettings::Grpc(GrpcSettings {
                service_name,
                multi_mode,
            }))
        }
        "h2" | "http" => {
            let http = &stream["httpSettings"];
            let host = http["host"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let path = http["path"].as_str().unwrap_or_default().to_string();
            Ok(TransportSettings::H2(H2Settings { host, path }))
        }
        "xhttp" | "splithttp" => {
            let xhttp = &stream["xhttpSettings"];
            let path = xhttp["path"].as_str().unwrap_or_default().to_string();
            let host = xhttp["host"].as_str().map(str::to_string);
            let mode = xhttp["mode"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("auto")
                .to_string();
            Ok(TransportSettings::Xhttp(XhttpSettings { path, host, mode }))
        }
        other => Err(format!("unsupported transport: {other}")),
    }
}

fn map_tls(stream: &Value) -> Result<Option<TlsSettings>, String> {
    let Some(security) = stream.get("security").and_then(Value::as_str) else {
        return Ok(None);
    };
    match security {
        "tls" => {
            let t = &stream["tlsSettings"];
            if t["allowInsecure"].as_bool().unwrap_or(false) {
                return Err("allowInsecure disables TLS certificate verification".into());
            }
            Ok(Some(TlsSettings {
                server_name: t["serverName"].as_str().map(str::to_string),
                alpn: t["alpn"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                verify: true,
                fingerprint: t["fingerprint"].as_str().map(str::to_string),
                reality: false,
                public_key: None,
                short_id: None,
                spider_x: None,
            }))
        }
        "reality" => {
            let r = &stream["realitySettings"];
            Ok(Some(TlsSettings {
                server_name: r["serverName"].as_str().map(str::to_string),
                alpn: Vec::new(),
                verify: true,
                fingerprint: r["fingerprint"].as_str().map(str::to_string),
                reality: true,
                public_key: r["publicKey"].as_str().map(str::to_string),
                short_id: r["shortId"].as_str().map(str::to_string),
                spider_x: r["spiderX"].as_str().map(str::to_string),
            }))
        }
        _ => Ok(None),
    }
}

fn outbound_actions(outbounds: &[Value]) -> HashMap<String, RuleAction> {
    let mut actions = HashMap::new();
    for outbound in outbounds {
        let Some(tag) = outbound.get("tag").and_then(Value::as_str) else {
            continue;
        };
        let protocol = outbound
            .get("protocol")
            .and_then(Value::as_str)
            .unwrap_or("");
        let action = match protocol {
            "freedom" => RuleAction::Direct,
            "blackhole" => RuleAction::Block,
            // DNS hijack is app-owned; a provider's dns-out tag has no routing
            // equivalent, so rules targeting it are skipped instead of widened.
            p if !NON_PROXY_PROTOCOLS.contains(&p) => RuleAction::Proxy,
            _ => continue,
        };
        actions.insert(tag.to_string(), action);
    }
    actions
}

/// Xray rule fields with no single-condition equivalent in the routing model,
/// or a meaning that would change silently when translated.
fn unsupported_rule_match(rule: &Value) -> Option<String> {
    if let Some(kind) = rule.get("type").and_then(Value::as_str)
        && kind != "field"
    {
        return Some(format!("unsupported rule type '{kind}'"));
    }
    for key in ["inboundTag", "source", "sourcePort", "user", "attrs"] {
        if rule.get(key).is_some() {
            return Some(format!("unsupported matcher '{key}'"));
        }
    }
    if rule
        .get("domain")
        .and_then(Value::as_array)
        .is_some_and(|domains| {
            domains
                .iter()
                .filter_map(Value::as_str)
                .any(|d| d.starts_with("regexp:"))
        })
    {
        return Some("unsupported 'regexp:' domain matcher".into());
    }
    None
}

/// `tcp,udp` selects every transport, so it adds no condition of its own.
fn network_is_tautological(spec: &str) -> bool {
    let mut saw_tcp = false;
    let mut saw_udp = false;
    let mut any = false;
    for part in spec.split(',').map(str::trim) {
        any = true;
        match part.to_ascii_lowercase().as_str() {
            "tcp" => saw_tcp = true,
            "udp" => saw_udp = true,
            _ => return false,
        }
    }
    any && saw_tcp && saw_udp
}

fn rule_axis_count(rule: &Value) -> usize {
    let has_strings = |key: &str| {
        rule.get(key)
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(Value::is_string))
    };
    let mut axes = 0;
    if has_strings("domain") {
        axes += 1;
    }
    if has_strings("ip") {
        axes += 1;
    }
    if has_strings("protocol") {
        axes += 1;
    }
    if rule.get("port").and_then(port_spec).is_some() {
        axes += 1;
    }
    if rule
        .get("network")
        .and_then(Value::as_str)
        .is_some_and(|spec| !network_is_tautological(spec))
    {
        axes += 1;
    }
    axes
}

fn map_routing_rules(routing: &Value, outbounds: &[Value]) -> (Vec<RoutingRule>, Vec<String>) {
    let mut rules = Vec::new();
    let mut skipped = Vec::new();
    let actions = outbound_actions(outbounds);

    let Some(raw_rules) = routing.get("rules").and_then(Value::as_array) else {
        return (rules, skipped);
    };

    for (i, rule) in raw_rules.iter().enumerate() {
        if let Some(reason) = unsupported_rule_match(rule) {
            skipped.push(format!("routing rule {i}: {reason}, dropped"));
            continue;
        }
        let axes = rule_axis_count(rule);
        if axes > 1 {
            skipped.push(format!(
                "routing rule {i}: combines {axes} match conditions, dropped"
            ));
            continue;
        }
        let Some(tag) = rule.get("outboundTag").and_then(Value::as_str) else {
            skipped.push(format!("routing rule {i}: missing outboundTag, ignored"));
            continue;
        };
        let Some(&action) = actions.get(tag) else {
            skipped.push(format!(
                "routing rule {i}: unknown outboundTag '{tag}', ignored"
            ));
            continue;
        };

        let mut matched_any = false;

        if let Some(domains) = rule.get("domain").and_then(Value::as_array) {
            for d in domains.iter().filter_map(Value::as_str) {
                matched_any = true;
                match domain_rule_match(d) {
                    Some(m) => push_rule(&mut rules, &mut skipped, i, m, action),
                    None => skipped.push(format!(
                        "routing rule {i}: unsupported domain matcher '{d}', ignored"
                    )),
                }
            }
        }
        if let Some(ips) = rule.get("ip").and_then(Value::as_array) {
            for ip in ips.iter().filter_map(Value::as_str) {
                matched_any = true;
                match ip_rule_match(ip) {
                    Some(m) => push_rule(&mut rules, &mut skipped, i, m, action),
                    None => skipped.push(format!(
                        "routing rule {i}: invalid ip matcher '{ip}', ignored"
                    )),
                }
            }
        }
        if let Some(protocols) = rule.get("protocol").and_then(Value::as_array) {
            for p in protocols.iter().filter_map(Value::as_str) {
                matched_any = true;
                push_rule(
                    &mut rules,
                    &mut skipped,
                    i,
                    RuleMatch::Protocol {
                        name: p.to_string(),
                    },
                    action,
                );
            }
        }
        if let Some(spec) = rule.get("port").and_then(port_spec) {
            matched_any = true;
            push_rule(
                &mut rules,
                &mut skipped,
                i,
                RuleMatch::Port { spec },
                action,
            );
        }
        if let Some(spec) = rule.get("network").and_then(Value::as_str)
            && !network_is_tautological(spec)
        {
            matched_any = true;
            push_rule(
                &mut rules,
                &mut skipped,
                i,
                RuleMatch::Network {
                    spec: spec.to_string(),
                },
                action,
            );
        }

        if !matched_any {
            skipped.push(format!(
                "routing rule {i}: no supported match condition, ignored"
            ));
        }
    }

    (rules, skipped)
}

fn push_rule(
    rules: &mut Vec<RoutingRule>,
    skipped: &mut Vec<String>,
    idx: usize,
    match_condition: RuleMatch,
    action: RuleAction,
) {
    match validate_rule_match(&match_condition) {
        Ok(()) => rules.push(RoutingRule {
            id: Uuid::new_v4(),
            match_condition,
            action,
            enabled: true,
            group: None,
            via_node: None,
        }),
        Err(e) => skipped.push(format!("routing rule {idx}: {e}, ignored")),
    }
}

fn domain_rule_match(d: &str) -> Option<RuleMatch> {
    if let Some(rest) = d.strip_prefix("domain:") {
        Some(RuleMatch::Domain {
            pattern: rest.to_string(),
        })
    } else if let Some(rest) = d.strip_prefix("full:") {
        Some(RuleMatch::DomainFull {
            domain: rest.to_string(),
        })
    } else if let Some(rest) = d.strip_prefix("geosite:") {
        Some(RuleMatch::GeoSite {
            category: rest.to_string(),
        })
    } else if d.starts_with("regexp:") {
        None
    } else if let Some(rest) = d.strip_prefix("keyword:") {
        Some(RuleMatch::DomainKeyword {
            keyword: rest.to_string(),
        })
    } else {
        Some(RuleMatch::DomainKeyword {
            keyword: d.to_string(),
        })
    }
}

fn ip_rule_match(ip: &str) -> Option<RuleMatch> {
    if let Some(rest) = ip.strip_prefix("geoip:") {
        return Some(RuleMatch::GeoIp {
            country_code: rest.to_uppercase(),
        });
    }
    parse_cidr(ip).map(|cidr| RuleMatch::IpCidr { cidr })
}

fn parse_cidr(s: &str) -> Option<ipnet::IpNet> {
    if let Ok(net) = s.parse::<ipnet::IpNet>() {
        return Some(net);
    }
    s.parse::<std::net::IpAddr>().ok().map(|addr| match addr {
        std::net::IpAddr::V4(v4) => ipnet::IpNet::V4(ipnet::Ipv4Net::new(v4, 32).unwrap()),
        std::net::IpAddr::V6(v6) => ipnet::IpNet::V6(ipnet::Ipv6Net::new(v6, 128).unwrap()),
    })
}

fn port_spec(v: &Value) -> Option<String> {
    if let Some(n) = v.as_u64() {
        return Some(n.to_string());
    }
    v.as_str().map(str::to_string)
}

fn map_dns_config(dns: &Value) -> (Option<DnsConfig>, Vec<String>) {
    let mut skipped = Vec::new();
    let Some(servers) = dns.get("servers").and_then(Value::as_array) else {
        return (None, skipped);
    };

    let mut dns_servers = Vec::new();
    let mut rules = Vec::new();

    for (i, server) in servers.iter().enumerate() {
        let (raw_address, domains) = match server {
            Value::String(s) => (s.clone(), Vec::new()),
            Value::Object(_) => {
                let Some(addr) = server.get("address").and_then(Value::as_str) else {
                    skipped.push(format!("dns server {i}: missing address, ignored"));
                    continue;
                };
                let domains = server
                    .get("domains")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                (addr.to_string(), domains)
            }
            _ => {
                skipped.push(format!("dns server {i}: unsupported shape, ignored"));
                continue;
            }
        };

        let (protocol, address, port, detour) = match parse_xray_dns_server(&raw_address) {
            Ok(parsed) => parsed,
            Err(reason) => {
                skipped.push(format!(
                    "dns server {i} ('{raw_address}'): {reason}, ignored"
                ));
                continue;
            }
        };
        if !valid_dns_address(&address) {
            skipped.push(format!(
                "dns server {i}: invalid address '{raw_address}', ignored"
            ));
            continue;
        }
        let tag = format!("provider-{i}");

        for d in &domains {
            match dns_domain_rule_match(d) {
                Some(m) => match validate_dns_rule_match(&m) {
                    Ok(()) => rules.push(DnsRule {
                        match_condition: m,
                        server_tag: tag.clone(),
                    }),
                    Err(e) => skipped.push(format!(
                        "dns server {i}: invalid domain matcher '{d}': {e}, ignored"
                    )),
                },
                None => skipped.push(format!(
                    "dns server {i}: unsupported domain matcher '{d}', ignored"
                )),
            }
        }

        dns_servers.push(DnsServerConfig {
            tag,
            protocol,
            address,
            port,
            detour,
        });
    }

    if dns_servers.is_empty() {
        return (None, skipped);
    }

    let hosts: Vec<HostOverride> = dns
        .get("hosts")
        .and_then(Value::as_object)
        .map(|obj| {
            obj.iter()
                .filter_map(|(domain, ip)| {
                    let Some(ip) = ip.as_str() else {
                        skipped.push(format!("dns host '{domain}': non-string address, ignored"));
                        return None;
                    };
                    if !valid_host_domain(domain) || ip.parse::<std::net::IpAddr>().is_err() {
                        skipped.push(format!("dns host '{domain}': invalid entry, ignored"));
                        return None;
                    }
                    Some(HostOverride {
                        domain: domain.clone(),
                        ip: ip.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let strategy = dns
        .get("queryStrategy")
        .and_then(Value::as_str)
        .map(dns_strategy_from_str)
        .unwrap_or_default();

    if dns.get("skipFallback").is_some() {
        skipped.push("dns.skipFallback has no model equivalent, dropped".to_string());
    }

    (
        Some(DnsConfig {
            enabled: true,
            strategy,
            servers: dns_servers,
            rules,
            fakeip: Default::default(),
            disable_cache: false,
            client_subnet: None,
            hosts,
            use_custom_rules: true,
        }),
        skipped,
    )
}

fn dns_strategy_from_str(s: &str) -> DnsStrategy {
    match s {
        "UseIPv4" => DnsStrategy::Ipv4Only,
        "UseIPv6" => DnsStrategy::Ipv6Only,
        "UseIPv6v4" => DnsStrategy::PreferIpv6,
        _ => DnsStrategy::PreferIpv4,
    }
}

fn dns_domain_rule_match(d: &str) -> Option<DnsRuleMatch> {
    if let Some(rest) = d.strip_prefix("domain:") {
        Some(DnsRuleMatch::DomainSuffix {
            suffix: rest.to_string(),
        })
    } else if let Some(rest) = d.strip_prefix("full:") {
        Some(DnsRuleMatch::DomainFull {
            domain: rest.to_string(),
        })
    } else if let Some(rest) = d.strip_prefix("geosite:") {
        Some(DnsRuleMatch::GeoSite {
            category: rest.to_string(),
        })
    } else if d.starts_with("regexp:") {
        None
    } else if let Some(rest) = d.strip_prefix("keyword:") {
        Some(DnsRuleMatch::DomainKeyword {
            keyword: rest.to_string(),
        })
    } else {
        Some(DnsRuleMatch::DomainKeyword {
            keyword: d.to_string(),
        })
    }
}

fn parse_xray_dns_server(
    raw: &str,
) -> Result<(DnsProtocol, String, Option<u16>, Option<String>), String> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("localhost") || trimmed.eq_ignore_ascii_case("fakedns") {
        return Err("unsupported pseudo-server".to_string());
    }

    let (normalized, local_detour) = if let Some(rest) = trimmed.strip_prefix("https+local://") {
        (format!("https://{rest}"), Some("direct".to_string()))
    } else if let Some(rest) = trimmed.strip_prefix("quic+local://") {
        (format!("quic://{rest}"), Some("direct".to_string()))
    } else if let Some(rest) = trimmed.strip_prefix("tcp+local://") {
        (format!("tcp://{rest}"), Some("direct".to_string()))
    } else {
        (trimmed.to_string(), None)
    };

    if normalized.contains("://") {
        let parsed = url::Url::parse(&normalized).map_err(|_| "invalid server URL".to_string())?;
        let Some(host) = parsed.host_str() else {
            return Err("missing host in server URL".to_string());
        };
        let protocol = match parsed.scheme() {
            "https" => DnsProtocol::Doh,
            "h3" => DnsProtocol::H3,
            "tls" => DnsProtocol::Dot,
            "quic" => DnsProtocol::Doq,
            "tcp" => DnsProtocol::Tcp,
            other => return Err(format!("unsupported DNS scheme '{other}'")),
        };
        if matches!(protocol, DnsProtocol::Doh | DnsProtocol::H3)
            && !matches!(parsed.path(), "" | "/" | "/dns-query")
        {
            return Err(format!("unsupported custom DoH path '{}'", parsed.path()));
        }
        return Ok((
            protocol,
            crate::parser::normalize_host(host),
            parsed.port(),
            local_detour,
        ));
    }

    Ok((DnsProtocol::Udp, trimmed.to_string(), None, None))
}

fn valid_dns_address(address: &str) -> bool {
    if address.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    valid_host_domain(address)
}

fn valid_host_domain(domain: &str) -> bool {
    if domain.is_empty() || domain.len() > 253 || domain.starts_with('.') || domain.ends_with('.') {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels
        .iter()
        .all(|label| !label.is_empty() && label.chars().all(|c| c.is_ascii_digit()))
    {
        return false;
    }
    labels.iter().all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    })
}
