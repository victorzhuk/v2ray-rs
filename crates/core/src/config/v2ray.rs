use serde_json::{Value, json};

use crate::config::{ConfigError, ConfigGenerator};
use crate::models::{
    AppSettings, ConnectionNodeRef, DnsHijackMode, DnsProtocol, DnsRuleMatch, DnsServerConfig,
    DnsStrategy, GrpcSettings, H2Settings, ProxyNode, RoutingRule, RuleAction, RuleMatch,
    ShadowsocksConfig, TransportSettings, TrojanConfig, TunConfig, VlessConfig, VmessConfig,
    WsSettings, XhttpSettings,
};

pub struct V2rayGenerator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum V2rayFamilyBackend {
    V2ray,
    Xray,
}

impl ConfigGenerator for V2rayGenerator {
    fn generate(
        &self,
        nodes: &[ProxyNode],
        rules: &[RoutingRule],
        settings: &AppSettings,
    ) -> Result<Value, ConfigError> {
        if nodes.is_empty() {
            return Err(ConfigError::NoNodes);
        }
        Ok(generate_v2ray_family_config(
            nodes,
            rules,
            settings,
            V2rayFamilyBackend::V2ray,
        ))
    }
}

pub(crate) fn generate_v2ray_family_config(
    nodes: &[ProxyNode],
    rules: &[RoutingRule],
    settings: &AppSettings,
    dns_backend: V2rayFamilyBackend,
) -> Value {
    let first_proxy_tag = super::common::outbound_tag(&nodes[0], 0);
    let via_tags = via_outbound_tags(nodes, rules);
    let tun_xray = dns_backend == V2rayFamilyBackend::Xray && settings.tun.enabled;

    // Built before routing: whether any server is tagged for direct dispatch
    // decides whether routing needs the rule that carries that tag.
    let dns = (settings.dns.enabled || tun_xray)
        .then(|| build_dns_for_backend(nodes, rules, settings, dns_backend, tun_xray));
    let has_direct_dns = dns.as_ref().is_some_and(has_direct_dns_server);

    let mut config = json!({
        "log": { "loglevel": "warning" },
        // Partial policy is valid; omitted timers keep backend defaults.
        // Without it xray's stock connIdle (300s) kills long-idle streams.
        "policy": {
            "levels": { "0": { "connIdle": settings.idle_timeout_secs } }
        },
        "inbounds": build_inbounds(settings, dns_backend),
        "outbounds": build_outbounds(nodes),
        "routing": build_routing(
            rules,
            &first_proxy_tag,
            &via_tags,
            settings,
            dns_backend,
            has_direct_dns,
        ),
    });

    if let Some(dns) = dns {
        config["dns"] = dns;
    }

    if tun_xray {
        config["dns"]["tag"] = json!(DNS_INTERNAL_TAG);
        harden_tun_outbounds(&mut config, settings);
    }

    config
}

fn has_direct_dns_server(dns: &Value) -> bool {
    dns["servers"]
        .as_array()
        .is_some_and(|servers| servers.iter().any(|s| s["tag"] == json!(DNS_DIRECT_TAG)))
}

/// Inbound tag stamped on queries made by xray's built-in resolver via
/// `dns.tag`, matched by the routing rule that sends them through the proxy.
const DNS_INTERNAL_TAG: &str = "dns-internal";

/// Resolver of last resort for domains no configured server claims. Under TUN
/// it is reached through the proxy like every other built-in resolver query, so
/// answers come back geo-located to the exit node rather than to the ISP.
const FALLBACK_DNS: &str = "https://1.1.1.1/dns-query";

/// Second unrestricted resolver, so one unreachable endpoint is a slow lookup
/// rather than a total outage.
const FALLBACK_DNS_SECONDARY: &str = "https://8.8.8.8/dns-query";

/// Plain-UDP bootstrap endpoint, tried before the DoH one. Networks that block
/// DoH on 443 outright are exactly the networks this app is used on, and there
/// the encrypted endpoint can be routed correctly and still never answer.
const BOOTSTRAP_DNS_UDP: &str = "1.1.1.1";

/// Stamped on servers whose queries must leave through `direct`. xray has no
/// per-server detour, but a tagged server labels its own queries, which a
/// routing rule can then match by `inboundTag`.
const DNS_DIRECT_TAG: &str = "dns-direct";

/// xray still queries a `domains`-scoped server for everything else unless
/// `skipFallback` says otherwise, so a region-scoped resolver ends up answering
/// for the domains it was never meant to see. Scope those strictly and make sure
/// something unrestricted is left to answer the rest.
fn apply_dns_fallback_policy(servers: &mut Vec<Value>) {
    let mut unrestricted = false;
    for server in servers.iter_mut() {
        if server.get("domains").is_some() {
            server["skipFallback"] = json!(true);
        } else {
            unrestricted = true;
        }
    }
    if !unrestricted {
        servers.push(json!(FALLBACK_DNS));
    }
}

fn query_strategy_str(strategy: DnsStrategy) -> &'static str {
    match strategy {
        DnsStrategy::PreferIpv4 | DnsStrategy::Ipv4Only => "UseIPv4",
        DnsStrategy::PreferIpv6 | DnsStrategy::Ipv6Only => "UseIPv6",
    }
}

/// `freedom` with the default `AsIs` hands sniffed hostnames to the OS
/// resolver at dial time; `UseIP` forces the built-in (proxy-routed) resolver.
/// Under `Hijack`, a `dns` outbound answers TUN-captured udp/53 with that same
/// resolver.
fn harden_tun_outbounds(config: &mut Value, settings: &AppSettings) {
    let Some(outbounds) = config["outbounds"].as_array_mut() else {
        return;
    };
    for outbound in outbounds.iter_mut() {
        if outbound["protocol"] == "freedom" {
            outbound["settings"]["domainStrategy"] = json!("UseIP");
        }
    }
    if settings.tun.dns_hijack == DnsHijackMode::Hijack {
        outbounds.push(json!({
            "protocol": "dns",
            "tag": "dns-out",
            "settings": {},
        }));
    }
}

fn build_inbounds(settings: &AppSettings, backend: V2rayFamilyBackend) -> Value {
    let mut inbounds = vec![
        json!({
            "tag": "socks-in",
            "protocol": "socks",
            "listen": settings.listen_address,
            "port": settings.socks_port,
            "settings": { "udp": true },
        }),
        json!({
            "tag": "http-in",
            "protocol": "http",
            "listen": settings.listen_address,
            "port": settings.http_port,
        }),
    ];

    // Only xray has a native `tun` inbound; v2ray-core has none.
    if backend == V2rayFamilyBackend::Xray && settings.tun.enabled {
        inbounds.push(build_xray_tun_inbound(&settings.tun));
    }

    Value::Array(inbounds)
}

fn build_xray_tun_inbound(tun: &TunConfig) -> Value {
    json!({
        "tag": "tun-in",
        "protocol": "tun",
        "settings": {
            "name": tun.interface_name,
            "mtu": tun.mtu,
            "gateway": tun.addresses(),
            "autoOutboundsInterface": "auto",
        },
        "sniffing": {
            "enabled": true,
            "destOverride": ["http", "tls", "quic"],
        },
    })
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

fn build_outbound(node: &ProxyNode, tag: &str) -> Value {
    match node {
        ProxyNode::Vless(c) => build_vless_outbound(c, tag),
        ProxyNode::Vmess(c) => build_vmess_outbound(c, tag),
        ProxyNode::Shadowsocks(c) => build_ss_outbound(c, tag),
        ProxyNode::Trojan(c) => build_trojan_outbound(c, tag),
    }
}

/// Builds a single v2ray-family outbound for the given node and tag. Shared
/// with the xray probe config generator.
pub(crate) fn build_family_outbound(node: &ProxyNode, tag: &str) -> Value {
    build_outbound(node, tag)
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
        TransportSettings::Xhttp(xhttp) => {
            stream["network"] = json!("xhttp");
            stream["xhttpSettings"] = build_xhttp_settings(xhttp);
        }
    }

    if let Some(tls_cfg) = tls {
        if tls_cfg.reality {
            stream["security"] = json!("reality");
            let mut reality_obj = json!({});
            if let Some(sni) = &tls_cfg.server_name {
                reality_obj["serverName"] = json!(sni);
            }
            if let Some(fp) = &tls_cfg.fingerprint {
                reality_obj["fingerprint"] = json!(fp);
            }
            if let Some(pbk) = &tls_cfg.public_key {
                reality_obj["publicKey"] = json!(pbk);
            }
            if let Some(sid) = &tls_cfg.short_id {
                reality_obj["shortId"] = json!(sid);
            }
            if let Some(spx) = &tls_cfg.spider_x {
                reality_obj["spiderX"] = json!(spx);
            }
            stream["realitySettings"] = reality_obj;
        } else {
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

fn build_xhttp_settings(xhttp: &XhttpSettings) -> Value {
    let mut settings = json!({
        "path": xhttp.path,
        "mode": xhttp.mode,
    });
    if let Some(host) = &xhttp.host {
        settings["host"] = json!(host);
    }
    settings
}

/// Pairs each distinct `via_node` target with the outbound tag that carries it.
///
/// The connected node is always `nodes[0]`; the connection layer appends the
/// rules' `via_node` targets after it in first-appearance order among enabled
/// rules, and clears any it could not resolve. Rebuilding that order here is
/// what lets a rule name a node without threading node identities through the
/// generator API.
pub(crate) fn via_outbound_tags(
    nodes: &[ProxyNode],
    rules: &[RoutingRule],
) -> Vec<(ConnectionNodeRef, String)> {
    let mut order: Vec<ConnectionNodeRef> = Vec::new();
    for rule in rules.iter().filter(|r| r.enabled) {
        if let Some(node_ref) = rule.via_node
            && !order.contains(&node_ref)
        {
            order.push(node_ref);
        }
    }

    order
        .into_iter()
        .enumerate()
        .filter_map(|(i, node_ref)| {
            let index = i + 1;
            let node = nodes.get(index)?;
            Some((node_ref, super::common::outbound_tag(node, index)))
        })
        .collect()
}

pub(crate) fn proxy_tag_for(
    rule: &RoutingRule,
    first_proxy_tag: &str,
    via_tags: &[(ConnectionNodeRef, String)],
) -> String {
    rule.via_node
        .and_then(|node_ref| via_tags.iter().find(|(k, _)| *k == node_ref))
        .map(|(_, tag)| tag.clone())
        .unwrap_or_else(|| first_proxy_tag.to_string())
}

fn build_routing(
    rules: &[RoutingRule],
    first_proxy_tag: &str,
    via_tags: &[(ConnectionNodeRef, String)],
    settings: &AppSettings,
    backend: V2rayFamilyBackend,
    has_direct_dns: bool,
) -> Value {
    let enabled: Vec<&RoutingRule> = rules.iter().filter(|r| r.enabled).collect();

    let mut routing_rules: Vec<Value> = Vec::new();

    if backend == V2rayFamilyBackend::Xray && settings.tun.enabled {
        // First of all: the port-53 hijack below carries no inboundTag, so it
        // would swallow a direct plain-UDP resolver's own query and feed it
        // back into the resolver that issued it.
        if has_direct_dns {
            routing_rules.push(json!({
                "type": "field",
                "inboundTag": [DNS_DIRECT_TAG],
                "outboundTag": "direct",
            }));
        }
        routing_rules.push(json!({
            "type": "field",
            "inboundTag": [DNS_INTERNAL_TAG],
            "outboundTag": first_proxy_tag,
        }));
        // Exclusions outrank the port-53 hijack below: xray takes the first
        // matching rule, so a hijack rule placed first would swallow every
        // query — including the ones aimed at a split-horizon resolver the user
        // excluded precisely because only it holds those records.
        if !settings.tun.exclude_routes.is_empty() {
            routing_rules.push(json!({
                "type": "field",
                "ip": &settings.tun.exclude_routes,
                "outboundTag": "direct",
            }));
        }
        if !settings.tun.exclude_domains.is_empty() {
            routing_rules.push(json!({
                "type": "field",
                "domain": &settings.tun.exclude_domains,
                "outboundTag": "direct",
            }));
        }
        if settings.tun.dns_hijack == DnsHijackMode::Hijack {
            // Both transports: the route helper steers tcp/53 into the tunnel
            // alongside udp/53, and a resolver falling back to TCP on a
            // truncated answer must not get a different view of the world.
            routing_rules.push(json!({
                "type": "field",
                "network": "tcp,udp",
                "port": 53,
                "outboundTag": "dns-out",
            }));
        }
    }

    if enabled.is_empty() && routing_rules.is_empty() {
        return json!({
            "domainStrategy": "AsIs",
            "rules": [],
        });
    }

    let user_rules: Vec<Value> = enabled
        .iter()
        .map(|r| build_routing_rule(r, first_proxy_tag, via_tags))
        .collect();
    routing_rules.extend(user_rules);

    json!({
        "domainStrategy": "IPIfNonMatch",
        "rules": routing_rules,
    })
}

fn build_routing_rule(
    rule: &RoutingRule,
    first_proxy_tag: &str,
    via_tags: &[(ConnectionNodeRef, String)],
) -> Value {
    let outbound_tag = match rule.action {
        RuleAction::Proxy => proxy_tag_for(rule, first_proxy_tag, via_tags),
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
            "domain": [format!("domain:{pattern}")],
            "outboundTag": outbound_tag,
        }),
        RuleMatch::DomainKeyword { keyword } => json!({
            "type": "field",
            "domain": [keyword],
            "outboundTag": outbound_tag,
        }),
        RuleMatch::DomainFull { domain } => json!({
            "type": "field",
            "domain": [format!("full:{domain}")],
            "outboundTag": outbound_tag,
        }),
        RuleMatch::IpCidr { cidr } => json!({
            "type": "field",
            "ip": [cidr.to_string()],
            "outboundTag": outbound_tag,
        }),
        RuleMatch::Protocol { name } => json!({
            "type": "field",
            "protocol": [name],
            "outboundTag": outbound_tag,
        }),
        RuleMatch::Port { spec } => json!({
            "type": "field",
            "port": spec,
            "outboundTag": outbound_tag,
        }),
        RuleMatch::Network { spec } => json!({
            "type": "field",
            "network": spec,
            "outboundTag": outbound_tag,
        }),
    }
}

fn udp_port_for_v2ray(server: &DnsServerConfig) -> Option<u16> {
    if server.protocol == DnsProtocol::Udp {
        server
            .port
            .filter(|&p| p != DnsProtocol::Udp.default_port())
    } else {
        None
    }
}

#[cfg(test)]
fn build_dns(rules: &[RoutingRule], settings: &AppSettings) -> Value {
    build_dns_for_backend(&[], rules, settings, V2rayFamilyBackend::V2ray, false)
}

fn build_dns_for_backend(
    nodes: &[ProxyNode],
    rules: &[RoutingRule],
    settings: &AppSettings,
    backend: V2rayFamilyBackend,
    tun_xray: bool,
) -> Value {
    let mut dns_config = json!({});

    let mut servers = if settings.dns.enabled {
        build_user_dns_servers(rules, settings, backend, tun_xray)
    } else {
        // TUN must never lean on the OS resolver: sniffing destOverride wipes
        // the original IP and IPIfNonMatch feeds geoip rules, so poisoned ISP
        // answers would steer blocked domains into `direct` and get RST by DPI.
        vec![json!(FALLBACK_DNS), json!(FALLBACK_DNS_SECONDARY)]
    };

    let bootstrap = bootstrap_dns_servers(nodes, settings, tun_xray);
    servers.splice(0..0, bootstrap);

    if servers.is_empty() {
        servers.push(if tun_xray {
            json!(FALLBACK_DNS)
        } else {
            json!("localhost")
        });
    }

    if backend == V2rayFamilyBackend::Xray {
        apply_dns_fallback_policy(&mut servers);
    }

    dns_config["servers"] = json!(servers);

    dns_config["queryStrategy"] = json!(query_strategy_str(settings.dns.strategy));

    if let Some(hosts) = hosts_for_strategy(settings) {
        dns_config["hosts"] = hosts;
    }

    if settings.dns.disable_cache {
        dns_config["disableCache"] = json!(true);
    }

    if let Some(ref subnet) = settings.dns.client_subnet {
        dns_config["clientIp"] = json!(subnet);
    }

    dns_config
}

/// Resolvers for the names that must be answered before the tunnel carries
/// anything: the proxy's own hostname, and any DNS server addressed by one.
/// Tagged so routing sends them out through `direct` instead of the proxy they
/// are trying to reach, and tried in transport order — plain UDP first because
/// DoH on 443 is blocked on many of the networks this runs on, encrypted second
/// because UDP/53 is intercepted on others. Only the last carries `finalQuery`,
/// so the pair is tried in order and nothing falls back past it onto a resolver
/// that is only reachable through the proxy being resolved.
fn bootstrap_dns_servers(
    nodes: &[ProxyNode],
    settings: &AppSettings,
    tun_xray: bool,
) -> Vec<Value> {
    if !tun_xray {
        return Vec::new();
    }

    let domains = bootstrap_domains(nodes, settings);
    if domains.is_empty() {
        return Vec::new();
    }

    vec![
        json!({
            "tag": DNS_DIRECT_TAG,
            "address": BOOTSTRAP_DNS_UDP,
            "domains": &domains,
            "skipFallback": true,
        }),
        json!({
            "tag": DNS_DIRECT_TAG,
            "address": FALLBACK_DNS,
            "domains": &domains,
            "skipFallback": true,
            "finalQuery": true,
        }),
    ]
}

fn bootstrap_domains(nodes: &[ProxyNode], settings: &AppSettings) -> Vec<String> {
    let mut domains: Vec<String> = Vec::new();
    for node in nodes {
        push_bootstrap_domain(&mut domains, node.address());
    }
    if settings.dns.enabled {
        for server in &settings.dns.servers {
            push_bootstrap_domain(&mut domains, &server.address);
        }
    }
    domains
}

fn push_bootstrap_domain(domains: &mut Vec<String>, host: &str) {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return;
    }
    let entry = format!("full:{host}");
    if !domains.contains(&entry) {
        domains.push(entry);
    }
}

/// xray answers a `hosts` hit authoritatively, so an entry holding only
/// addresses the query strategy discards resolves to an empty answer instead of
/// falling through to the servers. Keep the usable family, drop the rest.
fn hosts_for_strategy(settings: &AppSettings) -> Option<Value> {
    if settings.dns.hosts.is_empty() {
        return None;
    }

    // Several entries may name the same domain (a proxy host resolves to a
    // pool). xray takes an array there, so collect rather than overwrite —
    // last-wins would throw away every address but one.
    let mut hosts: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut named: Vec<&str> = Vec::new();
    for host in &settings.dns.hosts {
        if host.ip.parse::<std::net::IpAddr>().is_err() {
            log::warn!(
                "host override {} -> {} is not an address; dropped",
                host.domain,
                host.ip
            );
            continue;
        }
        if !named.contains(&host.domain.as_str()) {
            named.push(&host.domain);
        }
        if !host.matches_strategy(settings.dns.strategy) {
            continue;
        }

        match hosts.get_mut(&host.domain) {
            Some(Value::Array(ips)) => ips.push(json!(host.ip)),
            Some(existing) => {
                *existing = json!([existing.clone(), json!(host.ip)]);
            }
            None => {
                hosts.insert(host.domain.clone(), json!(host.ip));
            }
        }
    }

    for domain in named.iter().filter(|d| !hosts.contains_key(**d)) {
        log::warn!("host override {domain} has no address the query strategy can use; dropped");
    }

    (!hosts.is_empty()).then_some(Value::Object(hosts))
}

/// xray has no per-server detour field. Only "direct" is expressible, and only
/// under TUN, where a routing rule can act on the server's tag.
fn direct_detour(server: &DnsServerConfig, tun_xray: bool) -> bool {
    tun_xray && server.detours_direct()
}

fn dns_server_entry(
    address: String,
    port: Option<u16>,
    domains: Option<Vec<String>>,
    direct: bool,
) -> Value {
    if port.is_none() && domains.is_none() && !direct {
        return json!(address);
    }

    let mut entry = json!({ "address": address });
    if let Some(port) = port {
        entry["port"] = json!(port);
    }
    if let Some(domains) = domains {
        entry["domains"] = json!(domains);
    }
    if direct {
        entry["tag"] = json!(DNS_DIRECT_TAG);
    }
    entry
}

fn build_user_dns_servers(
    rules: &[RoutingRule],
    settings: &AppSettings,
    backend: V2rayFamilyBackend,
    tun_xray: bool,
) -> Vec<Value> {
    let mut servers: Vec<Value> = Vec::new();

    if settings.dns.use_custom_rules {
        for server in &settings.dns.servers {
            let domains: Vec<String> = settings
                .dns
                .rules
                .iter()
                .filter(|rule| rule.server_tag == server.tag)
                .map(|rule| match &rule.match_condition {
                    DnsRuleMatch::GeoSite { category } => format!("geosite:{category}"),
                    DnsRuleMatch::DomainSuffix { suffix } => format!("domain:{suffix}"),
                    DnsRuleMatch::DomainKeyword { keyword } => keyword.clone(),
                    DnsRuleMatch::DomainFull { domain } => format!("full:{domain}"),
                })
                .collect();

            servers.push(dns_server_entry(
                dns_server_address_for_backend(server, backend),
                udp_port_for_v2ray(server),
                (!domains.is_empty()).then_some(domains),
                direct_detour(server, tun_xray),
            ));
        }
    } else {
        let mut remote_domains: Vec<String> = Vec::new();
        let mut domestic_domains: Vec<String> = Vec::new();

        for rule in rules.iter().filter(|r| r.enabled) {
            let entry = match &rule.match_condition {
                RuleMatch::GeoSite { category } => Some(format!("geosite:{category}")),
                RuleMatch::Domain { pattern } => Some(format!("domain:{pattern}")),
                RuleMatch::DomainKeyword { keyword } => Some(keyword.clone()),
                RuleMatch::DomainFull { domain } => Some(format!("full:{domain}")),
                _ => None,
            };
            if let Some(d) = entry {
                match rule.action {
                    RuleAction::Proxy => remote_domains.push(d),
                    RuleAction::Direct => domestic_domains.push(d),
                    RuleAction::Block => {}
                }
            }
        }

        if backend == V2rayFamilyBackend::Xray
            && settings.tun.enabled
            && !settings.tun.exclude_domains.is_empty()
        {
            for d in &settings.tun.exclude_domains {
                domestic_domains.push(d.clone());
            }
        }

        for server in &settings.dns.servers {
            let domains = match server.tag.as_str() {
                "remote" if !remote_domains.is_empty() => Some(remote_domains.clone()),
                "domestic" if !domestic_domains.is_empty() => Some(domestic_domains.clone()),
                _ => None,
            };
            servers.push(dns_server_entry(
                dns_server_address_for_backend(server, backend),
                udp_port_for_v2ray(server),
                domains,
                direct_detour(server, tun_xray),
            ));
        }
    }

    if backend == V2rayFamilyBackend::Xray
        && settings.tun.enabled
        && !settings.tun.exclude_domains.is_empty()
        && settings.dns.use_custom_rules
    {
        attach_exclude_domains(&mut servers, settings, backend);
    }

    servers
}

/// Excluded domains are split-horizon names, so they are folded into the
/// server that resolves outside the tunnel when there is one, and the OS
/// resolver otherwise.
fn attach_exclude_domains(
    servers: &mut Vec<Value>,
    settings: &AppSettings,
    backend: V2rayFamilyBackend,
) {
    let Some(target) = super::common::split_horizon_server(settings) else {
        servers.push(json!({
            "address": "localhost",
            "domains": &settings.tun.exclude_domains,
        }));
        return;
    };
    let target_addr = dns_server_address_for_backend(target, backend);

    let Some(entry) = servers.iter_mut().find(|s| {
        s.as_str().map(|a| a == target_addr).unwrap_or_else(|| {
            s.get("address")
                .and_then(|a| a.as_str())
                .map(|a| a == target_addr)
                .unwrap_or(false)
        })
    }) else {
        return;
    };

    if entry.is_string() {
        let addr = entry.as_str().unwrap_or_default().to_string();
        *entry = json!({ "address": addr, "domains": &settings.tun.exclude_domains });
    } else if let Some(Value::Array(domains)) = entry.get_mut("domains") {
        for d in &settings.tun.exclude_domains {
            domains.push(json!(d));
        }
    } else {
        entry["domains"] = json!(&settings.tun.exclude_domains);
    }
}

fn dns_server_address_for_backend(server: &DnsServerConfig, backend: V2rayFamilyBackend) -> String {
    let backend_type = match backend {
        V2rayFamilyBackend::V2ray => crate::models::BackendType::V2ray,
        V2rayFamilyBackend::Xray => crate::models::BackendType::Xray,
    };
    let effective_protocol = server.protocol.effective_for_backend(backend_type);

    if effective_protocol != server.protocol {
        let backend_name = match backend {
            V2rayFamilyBackend::V2ray => "v2ray",
            V2rayFamilyBackend::Xray => "xray",
        };
        log::warn!(
            "{backend_name} does not support {:?} DNS for '{}' directly; falling back to {:?}",
            server.protocol,
            server.tag,
            effective_protocol
        );
    }

    effective_protocol.server_address(&server.address, server.port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_fixtures::fixtures::*;
    use crate::models::{DnsServerConfig, DnsStrategy, HostOverride, *};
    use uuid::Uuid;

    #[test]
    fn test_generate_returns_error_on_empty_nodes() {
        let generator = V2rayGenerator;
        let result = generator.generate(&[], &[], &default_settings());
        assert!(result.is_err());
    }

    #[test]
    fn test_basic_vless_config_structure() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[vless_node()], &[], &default_settings())
            .unwrap();

        assert!(config["log"].is_object());
        assert!(config["inbounds"].is_array());
        assert!(config["outbounds"].is_array());
        assert!(config["routing"].is_object());
    }

    #[test]
    fn test_policy_conn_idle_from_settings() {
        let mut settings = default_settings();
        settings.idle_timeout_secs = 900;

        for backend in [V2rayFamilyBackend::V2ray, V2rayFamilyBackend::Xray] {
            let config = generate_v2ray_family_config(&[vless_node()], &[], &settings, backend);
            assert_eq!(config["policy"]["levels"]["0"]["connIdle"], 900);
        }
    }

    fn proxy_rule(pattern: &str, via_node: Option<ConnectionNodeRef>) -> RoutingRule {
        RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::Domain {
                pattern: pattern.into(),
            },
            action: RuleAction::Proxy,
            enabled: true,
            group: None,
            via_node,
        }
    }

    #[test]
    fn test_rule_via_node_targets_that_nodes_outbound() {
        let pinned = ConnectionNodeRef::Manual {
            node_id: Uuid::new_v4(),
        };
        let nodes = [vless_node(), ss_node()];
        let rules = vec![
            proxy_rule("api.z.ai", Some(pinned)),
            proxy_rule("example.org", None),
        ];

        let config = generate_v2ray_family_config(
            &nodes,
            &rules,
            &default_settings(),
            V2rayFamilyBackend::Xray,
        );

        let routing = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(
            routing[0]["outboundTag"],
            crate::config::common::outbound_tag(&nodes[1], 1)
        );
        assert_eq!(
            routing[1]["outboundTag"],
            crate::config::common::outbound_tag(&nodes[0], 0)
        );
    }

    #[test]
    fn test_rule_via_node_falls_back_when_node_missing() {
        let pinned = ConnectionNodeRef::Manual {
            node_id: Uuid::new_v4(),
        };
        // Only the connected node was passed, so the pinned ref has no outbound.
        let nodes = [vless_node()];
        let rules = vec![proxy_rule("api.z.ai", Some(pinned))];

        let config = generate_v2ray_family_config(
            &nodes,
            &rules,
            &default_settings(),
            V2rayFamilyBackend::Xray,
        );

        let routing = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(
            routing[0]["outboundTag"],
            crate::config::common::outbound_tag(&nodes[0], 0)
        );
    }

    #[test]
    fn test_via_outbound_tags_dedupes_and_keeps_first_appearance_order() {
        let a = ConnectionNodeRef::Manual {
            node_id: Uuid::new_v4(),
        };
        let b = ConnectionNodeRef::Manual {
            node_id: Uuid::new_v4(),
        };
        let nodes = [vless_node(), ss_node(), trojan_node()];
        let rules = vec![
            proxy_rule("one.example", Some(a)),
            proxy_rule("two.example", Some(b)),
            proxy_rule("three.example", Some(a)),
        ];

        let tags = via_outbound_tags(&nodes, &rules);

        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].0, a);
        assert_eq!(tags[1].0, b);
        assert_eq!(tags[0].1, crate::config::common::outbound_tag(&nodes[1], 1));
        assert_eq!(tags[1].1, crate::config::common::outbound_tag(&nodes[2], 2));
    }

    #[test]
    fn test_disabled_rule_does_not_claim_an_outbound_slot() {
        let pinned = ConnectionNodeRef::Manual {
            node_id: Uuid::new_v4(),
        };
        let mut disabled = proxy_rule("skipped.example", Some(pinned));
        disabled.enabled = false;
        let live = ConnectionNodeRef::Manual {
            node_id: Uuid::new_v4(),
        };
        let nodes = [vless_node(), ss_node()];
        let rules = vec![disabled, proxy_rule("api.z.ai", Some(live))];

        let tags = via_outbound_tags(&nodes, &rules);

        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].0, live);
        assert_eq!(tags[0].1, crate::config::common::outbound_tag(&nodes[1], 1));
    }

    #[test]
    fn test_repeated_host_domain_becomes_an_address_array() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.hosts = vec![
            HostOverride {
                domain: "ap.example.com".into(),
                ip: "203.0.113.1".into(),
            },
            HostOverride {
                domain: "ap.example.com".into(),
                ip: "203.0.113.2".into(),
            },
            HostOverride {
                domain: "single.example.com".into(),
                ip: "203.0.113.9".into(),
            },
        ];

        let config =
            generate_v2ray_family_config(&[vless_node()], &[], &settings, V2rayFamilyBackend::Xray);

        assert_eq!(
            config["dns"]["hosts"]["ap.example.com"],
            json!(["203.0.113.1", "203.0.113.2"]),
            "a pooled proxy host must keep every address, not just the last"
        );
        assert_eq!(config["dns"]["hosts"]["single.example.com"], "203.0.113.9");
    }

    fn scoped_dns_settings() -> AppSettings {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.use_custom_rules = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "domestic".into(),
            protocol: DnsProtocol::Udp,
            address: "192.0.2.1".into(),
            port: None,
            detour: None,
        }];
        settings.dns.rules = vec![DnsRule {
            match_condition: DnsRuleMatch::GeoSite {
                category: "category-ru".into(),
            },
            server_tag: "domestic".into(),
        }];
        settings
    }

    #[test]
    fn test_scoped_dns_server_skips_fallback_and_gains_a_catch_all() {
        let config = generate_v2ray_family_config(
            &[vless_node()],
            &[],
            &scoped_dns_settings(),
            V2rayFamilyBackend::Xray,
        );

        let servers = config["dns"]["servers"].as_array().unwrap();
        let scoped = &servers[0];
        assert_eq!(scoped["address"], "192.0.2.1");
        assert_eq!(scoped["skipFallback"], true);
        assert_eq!(
            servers.last().unwrap(),
            FALLBACK_DNS,
            "a scoped-only server list needs an unrestricted resolver: {servers:?}"
        );
    }

    #[test]
    fn test_unrestricted_dns_server_keeps_fallback_and_adds_nothing() {
        let mut settings = scoped_dns_settings();
        settings.dns.rules.clear();

        let config =
            generate_v2ray_family_config(&[vless_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let servers = config["dns"]["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 1, "no catch-all needed: {servers:?}");
        assert_eq!(servers[0], "192.0.2.1");
    }

    #[test]
    fn test_dns_fallback_policy_is_xray_only() {
        let config = generate_v2ray_family_config(
            &[vless_node()],
            &[],
            &scoped_dns_settings(),
            V2rayFamilyBackend::V2ray,
        );

        let servers = config["dns"]["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 1);
        assert!(servers[0].get("skipFallback").is_none());
    }

    fn find_tun_inbound(config: &Value) -> Option<&Value> {
        config["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["protocol"] == "tun")
    }

    #[test]
    fn test_xray_tun_inbound_emitted_when_enabled() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.address_v4 = "198.18.0.1/30".to_string();

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let tun = find_tun_inbound(&config).expect("xray tun inbound missing");
        assert_eq!(tun["protocol"], "tun");
        assert_eq!(tun["settings"]["name"], "tun0");
        assert_eq!(tun["settings"]["mtu"], 1500);
        assert_eq!(tun["settings"]["gateway"], json!(["198.18.0.1/30"]));
        assert_eq!(tun["settings"]["autoOutboundsInterface"], "auto");
        assert_eq!(tun["sniffing"]["enabled"], true);
    }

    #[test]
    fn test_xray_no_tun_inbound_when_disabled() {
        let config = generate_v2ray_family_config(
            &[ss_node()],
            &[],
            &default_settings(),
            V2rayFamilyBackend::Xray,
        );
        assert!(find_tun_inbound(&config).is_none());
    }

    #[test]
    fn test_v2ray_never_emits_tun_even_when_enabled() {
        let mut settings = default_settings();
        settings.tun.enabled = true;

        let config = V2rayGenerator
            .generate(&[ss_node()], &[], &settings)
            .unwrap();

        assert!(find_tun_inbound(&config).is_none());
        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 2);
    }

    #[test]
    fn test_inbound_ports() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[vless_node()], &[], &default_settings())
            .unwrap();

        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 2);
        assert_eq!(inbounds[0]["port"], 1080);
        assert_eq!(inbounds[0]["protocol"], "socks");
        assert_eq!(inbounds[1]["port"], 1081);
        assert_eq!(inbounds[1]["protocol"], "http");
    }

    #[test]
    fn test_inbound_listen_address_default_loopback() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[vless_node()], &[], &default_settings())
            .unwrap();

        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds[0]["listen"], "127.0.0.1");
        assert_eq!(inbounds[1]["listen"], "127.0.0.1");
    }

    #[test]
    fn test_inbound_listen_address_from_settings() {
        let generator = V2rayGenerator;
        let mut settings = default_settings();
        settings.listen_address = "0.0.0.0".to_string();
        let config = generator.generate(&[vless_node()], &[], &settings).unwrap();

        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds[0]["listen"], "0.0.0.0");
        assert_eq!(inbounds[1]["listen"], "0.0.0.0");
        // ports unchanged
        assert_eq!(inbounds[0]["port"], 1080);
        assert_eq!(inbounds[1]["port"], 1081);
    }

    #[test]
    fn test_socks_inbound_udp_enabled() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[vless_node()], &[], &default_settings())
            .unwrap();

        assert_eq!(config["inbounds"][0]["settings"]["udp"], true);
    }

    #[test]
    fn test_xray_inbound_listen_address_from_settings() {
        // The xray generator reuses the v2ray family code path; assert the
        // setting propagates when generating an xray-flavoured config.
        let mut settings = default_settings();
        settings.listen_address = "0.0.0.0".to_string();
        let config =
            generate_v2ray_family_config(&[vless_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let inbounds = config["inbounds"].as_array().unwrap();
        assert_eq!(inbounds[0]["listen"], "0.0.0.0");
        assert_eq!(inbounds[1]["listen"], "0.0.0.0");
        assert_eq!(inbounds[0]["settings"]["udp"], true);
    }

    #[test]
    fn test_vless_outbound() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[vless_node()], &[], &default_settings())
            .unwrap();

        let outbounds = config["outbounds"].as_array().unwrap();
        let proxy = &outbounds[0];
        assert_eq!(proxy["protocol"], "vless");
        assert_eq!(proxy["settings"]["vnext"][0]["address"], "example.com");
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
            .generate(&[vmess_node()], &[], &default_settings())
            .unwrap();

        let proxy = &config["outbounds"][0];
        assert_eq!(proxy["protocol"], "vmess");
        assert_eq!(
            proxy["settings"]["vnext"][0]["users"][0]["security"],
            "auto"
        );
        assert_eq!(proxy["settings"]["vnext"][0]["users"][0]["alterId"], 0);
    }

    #[test]
    fn test_shadowsocks_outbound() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[ss_node()], &[], &default_settings())
            .unwrap();

        let proxy = &config["outbounds"][0];
        assert_eq!(proxy["protocol"], "shadowsocks");
        assert_eq!(proxy["settings"]["servers"][0]["method"], "aes-256-gcm");
    }

    #[test]
    fn test_trojan_outbound() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[trojan_node()], &[], &default_settings())
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
            .generate(&[vless_node()], &[], &default_settings())
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
        let config = generator
            .generate(&nodes, &[], &default_settings())
            .unwrap();

        let outbounds = config["outbounds"].as_array().unwrap();
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
            group: None,
            via_node: None,
        }];

        let config = generator
            .generate(&[vless_node()], &rules, &default_settings())
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
            group: None,
            via_node: None,
        }];

        let config = generator
            .generate(&[vless_node()], &rules, &default_settings())
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
            group: None,
            via_node: None,
        }];

        let config = generator
            .generate(&[vless_node()], &rules, &default_settings())
            .unwrap();

        let routing_rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(routing_rules[0]["domain"][0], "domain:*.google.com");
    }

    #[test]
    fn test_domain_keyword_routing_rule() {
        let generator = V2rayGenerator;
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
            .generate(&[vless_node()], &rules, &default_settings())
            .unwrap();

        let routing_rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(routing_rules[0]["domain"][0], "sina");
    }

    #[test]
    fn test_domain_full_routing_rule() {
        let generator = V2rayGenerator;
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
            .generate(&[vless_node()], &rules, &default_settings())
            .unwrap();

        let routing_rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(routing_rules[0]["domain"][0], "full:example.com");
    }

    #[test]
    fn test_protocol_routing_rule() {
        let generator = V2rayGenerator;
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
            .generate(&[vless_node()], &rules, &default_settings())
            .unwrap();

        let routing_rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(routing_rules[0]["protocol"][0], "bittorrent");
        assert_eq!(routing_rules[0]["outboundTag"], "block");
    }

    #[test]
    fn test_port_routing_rule() {
        let generator = V2rayGenerator;
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
            .generate(&[vless_node()], &rules, &default_settings())
            .unwrap();

        let routing_rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(routing_rules[0]["port"], "80,443,1000-2000");
    }

    #[test]
    fn test_network_routing_rule() {
        let generator = V2rayGenerator;
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
            .generate(&[vless_node()], &rules, &default_settings())
            .unwrap();

        let routing_rules = config["routing"]["rules"].as_array().unwrap();
        assert_eq!(routing_rules[0]["network"], "tcp,udp");
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
            group: None,
            via_node: None,
        }];

        let config = generator
            .generate(&[vless_node()], &rules, &default_settings())
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
            .generate(&[vless_node()], &rules, &default_settings())
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
            .generate(&[node], &[], &default_settings())
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
            .generate(&[node], &[], &default_settings())
            .unwrap();

        let stream = &config["outbounds"][0]["streamSettings"];
        assert_eq!(stream["network"], "h2");
        assert_eq!(stream["httpSettings"]["path"], "/h2path");
    }

    #[test]
    fn test_xhttp_transport() {
        let generator = V2rayGenerator;
        let config = generator
            .generate(&[xhttp_node()], &[], &default_settings())
            .unwrap();

        let stream = &config["outbounds"][0]["streamSettings"];
        assert_eq!(stream["network"], "xhttp");
        assert_eq!(stream["xhttpSettings"]["path"], "/xhttp");
        assert_eq!(stream["xhttpSettings"]["host"], "xhttp.example.com");
        assert_eq!(stream["xhttpSettings"]["mode"], "auto");
        assert_eq!(stream["security"], "reality");
    }

    #[test]
    fn test_xray_xhttp_transport() {
        let generator = crate::config::XrayGenerator;
        let config = generator
            .generate(&[xhttp_node()], &[], &default_settings())
            .unwrap();

        let stream = &config["outbounds"][0]["streamSettings"];
        assert_eq!(stream["network"], "xhttp");
        assert_eq!(stream["xhttpSettings"]["mode"], "auto");
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
            .generate(&nodes, &rules, &default_settings())
            .unwrap();
        let json_str = serde_json::to_string_pretty(&config).unwrap();
        let _: Value = serde_json::from_str(&json_str).unwrap();
    }

    #[test]
    fn test_dns_multiple_servers() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![
            DnsServerConfig {
                tag: "cloudflare".to_string(),
                protocol: DnsProtocol::Doh,
                address: "1.1.1.1".to_string(),
                port: None,
                detour: None,
            },
            DnsServerConfig {
                tag: "google".to_string(),
                protocol: DnsProtocol::Udp,
                address: "8.8.8.8".to_string(),
                port: Some(5353),
                detour: None,
            },
        ];

        let dns = build_dns(&[], &settings);
        let servers = dns["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].as_str(), Some("https://1.1.1.1/dns-query"));
        assert_eq!(servers[1]["address"].as_str(), Some("8.8.8.8"));
        assert_eq!(servers[1]["port"], 5353);
    }

    #[test]
    fn test_dns_query_strategy_mapping() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "test".to_string(),
            protocol: DnsProtocol::Udp,
            address: "8.8.8.8".to_string(),
            port: None,
            detour: None,
        }];

        let strategies = vec![
            (DnsStrategy::PreferIpv4, "UseIPv4"),
            (DnsStrategy::PreferIpv6, "UseIPv6"),
            (DnsStrategy::Ipv4Only, "UseIPv4"),
            (DnsStrategy::Ipv6Only, "UseIPv6"),
        ];

        for (strategy, expected) in strategies {
            settings.dns.strategy = strategy;
            let dns = build_dns(&[], &settings);
            assert_eq!(dns["queryStrategy"], expected);
        }
    }

    #[test]
    fn test_dns_hosts_generation() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "test".to_string(),
            protocol: DnsProtocol::Udp,
            address: "8.8.8.8".to_string(),
            port: None,
            detour: None,
        }];
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

        let dns = build_dns(&[], &settings);
        let hosts = dns["hosts"].as_object().unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts.get("example.com"), Some(&json!("192.0.2.1")));
        assert_eq!(hosts.get("test.local"), Some(&json!("10.0.0.1")));
    }

    #[test]
    fn test_dns_supported_protocol_addresses_preserved() {
        let mut settings = default_settings();
        settings.dns.enabled = true;

        let protocols = vec![
            (DnsProtocol::Doh, "https://1.1.1.1/dns-query"),
            (DnsProtocol::Tcp, "tcp://1.1.1.1:53"),
            (DnsProtocol::Udp, "1.1.1.1"),
        ];

        for (protocol, expected_address) in protocols {
            settings.dns.servers = vec![DnsServerConfig {
                tag: "test".to_string(),
                protocol,
                address: "1.1.1.1".to_string(),
                port: None,
                detour: None,
            }];

            let dns = build_dns(&[], &settings);
            let servers = dns["servers"].as_array().unwrap();
            assert_eq!(servers[0].as_str(), Some(expected_address));
        }
    }

    #[test]
    fn test_dns_v2ray_falls_back_to_doh_for_unsupported_protocols() {
        let mut settings = default_settings();
        settings.dns.enabled = true;

        let protocols = vec![
            (DnsProtocol::Dot, "https://1.1.1.1/dns-query"),
            (DnsProtocol::Doq, "https://1.1.1.1/dns-query"),
            (DnsProtocol::H3, "https://1.1.1.1/dns-query"),
        ];

        for (protocol, expected_address) in protocols {
            settings.dns.servers = vec![DnsServerConfig {
                tag: "test".to_string(),
                protocol,
                address: "1.1.1.1".to_string(),
                port: None,
                detour: None,
            }];

            let dns = build_dns(&[], &settings);
            let servers = dns["servers"].as_array().unwrap();
            assert_eq!(servers[0].as_str(), Some(expected_address));
        }
    }

    #[test]
    fn test_dns_disable_cache_and_client_subnet() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "test".to_string(),
            protocol: DnsProtocol::Udp,
            address: "8.8.8.8".to_string(),
            port: None,
            detour: None,
        }];
        settings.dns.disable_cache = true;
        settings.dns.client_subnet = Some("203.0.113.1".to_string());

        let dns = build_dns(&[], &settings);
        assert_eq!(dns["disableCache"], true);
        assert_eq!(dns["clientIp"], "203.0.113.1");
    }

    #[test]
    fn test_dns_custom_rules_with_per_server_domains() {
        use crate::models::{DnsRule, DnsRuleMatch};

        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![
            DnsServerConfig {
                tag: "remote".to_string(),
                protocol: DnsProtocol::Doh,
                address: "1.1.1.1".to_string(),
                port: None,
                detour: None,
            },
            DnsServerConfig {
                tag: "domestic".to_string(),
                protocol: DnsProtocol::Udp,
                address: "223.5.5.5".to_string(),
                port: None,
                detour: None,
            },
        ];
        settings.dns.use_custom_rules = true;
        settings.dns.rules = vec![
            DnsRule {
                match_condition: DnsRuleMatch::GeoSite {
                    category: "google".to_string(),
                },
                server_tag: "remote".to_string(),
            },
            DnsRule {
                match_condition: DnsRuleMatch::DomainSuffix {
                    suffix: ".cn".to_string(),
                },
                server_tag: "domestic".to_string(),
            },
        ];

        let dns = build_dns(&[], &settings);
        let servers = dns["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 2);

        let remote = &servers[0];
        assert_eq!(remote["address"], "https://1.1.1.1/dns-query");
        let remote_domains = remote["domains"].as_array().unwrap();
        assert_eq!(remote_domains.len(), 1);
        assert_eq!(remote_domains[0], "geosite:google");

        let domestic = &servers[1];
        assert_eq!(domestic["address"], "223.5.5.5");
        let domestic_domains = domestic["domains"].as_array().unwrap();
        assert_eq!(domestic_domains.len(), 1);
        assert_eq!(domestic_domains[0], "domain:.cn");
    }

    #[test]
    fn test_dns_custom_rules_domain_keyword_and_full() {
        use crate::models::{DnsRule, DnsRuleMatch};

        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "remote".to_string(),
            protocol: DnsProtocol::Doh,
            address: "1.1.1.1".to_string(),
            port: None,
            detour: None,
        }];
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
                server_tag: "remote".to_string(),
            },
        ];

        let dns = build_dns(&[], &settings);
        let servers = dns["servers"].as_array().unwrap();
        let domains = servers[0]["domains"].as_array().unwrap();
        assert!(domains.contains(&json!("sina")));
        assert!(domains.contains(&json!("full:example.com")));
    }

    #[test]
    fn test_dns_derived_domain_rule_gains_prefix() {
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

        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.use_custom_rules = false;

        let dns = build_dns(&rules, &settings);
        let servers = dns["servers"].as_array().unwrap();
        let remote = servers
            .iter()
            .find(|s| s.get("domains").is_some())
            .expect("remote server with derived domains not found");
        let domains = remote["domains"].as_array().unwrap();
        assert!(domains.contains(&json!("domain:example.com")));
    }

    #[test]
    fn test_dns_empty_servers_uses_localhost() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![];

        let dns = build_dns(&[], &settings);
        let servers = dns["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].as_str(), Some("localhost"));
    }

    #[test]
    fn test_dns_tcp_protocol_formatting() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "test".to_string(),
            protocol: DnsProtocol::Tcp,
            address: "8.8.8.8".to_string(),
            port: Some(5353),
            detour: None,
        }];

        let dns = build_dns(&[], &settings);
        let servers = dns["servers"].as_array().unwrap();
        assert_eq!(servers[0].as_str(), Some("tcp://8.8.8.8:5353"));
    }

    #[test]
    fn test_dns_full_config_valid_json() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.strategy = DnsStrategy::Ipv4Only;
        settings.dns.servers = vec![
            DnsServerConfig {
                tag: "cloudflare".to_string(),
                protocol: DnsProtocol::Doh,
                address: "1.1.1.1".to_string(),
                port: None,
                detour: None,
            },
            DnsServerConfig {
                tag: "google".to_string(),
                protocol: DnsProtocol::Udp,
                address: "8.8.8.8".to_string(),
                port: Some(5353),
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

        let generator = V2rayGenerator;
        let config = generator.generate(&[vless_node()], &[], &settings).unwrap();

        assert!(config.get("dns").is_some());
        let dns = &config["dns"];

        assert_eq!(dns["queryStrategy"], "UseIPv4");
        assert_eq!(dns["disableCache"], true);
        assert_eq!(dns["clientIp"], "203.0.113.1");

        let servers = dns["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 2);

        let hosts = dns["hosts"].as_object().unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts.get("example.com"), Some(&json!("192.0.2.1")));
        assert_eq!(hosts.get("test.local"), Some(&json!("10.0.0.1")));

        let json_str = serde_json::to_string(&config).unwrap();
        let _: Value = serde_json::from_str(&json_str).unwrap();
    }

    #[test]
    fn test_xray_tun_exclusion_ip_and_domain() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.exclude_routes = vec!["104.16.0.0/13".to_string()];
        settings.tun.exclude_domains = vec!["example.com".to_string()];

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let rules = config["routing"]["rules"].as_array().unwrap();
        assert!(rules.len() >= 2);

        let ip_rule = rules
            .iter()
            .find(|r| r.get("ip").is_some())
            .expect("ip exclusion rule not found");
        assert_eq!(ip_rule["type"], "field");
        assert_eq!(ip_rule["ip"], json!(["104.16.0.0/13"]));
        assert_eq!(ip_rule["outboundTag"], "direct");

        let domain_rule = rules
            .iter()
            .find(|r| r.get("domain").is_some())
            .expect("domain exclusion rule not found");
        assert_eq!(domain_rule["type"], "field");
        assert_eq!(domain_rule["domain"], json!(["example.com"]));
        assert_eq!(domain_rule["outboundTag"], "direct");
    }

    /// xray takes the first matching rule, so an exclusion listed after the
    /// port-53 hijack never applies to DNS — which is the only traffic an
    /// excluded split-horizon resolver exists to carry.
    #[test]
    fn test_xray_tun_exclusion_precedes_dns_hijack() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.dns_hijack = DnsHijackMode::Hijack;
        settings.tun.exclude_routes = vec!["10.15.12.100/32".to_string()];
        settings.tun.exclude_domains = vec!["example.com".to_string()];

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let rules = config["routing"]["rules"].as_array().unwrap();
        let hijack = rules
            .iter()
            .position(|r| r["outboundTag"] == "dns-out")
            .expect("dns hijack rule not found");
        let ip = rules
            .iter()
            .position(|r| r.get("ip").is_some())
            .expect("ip exclusion rule not found");
        let domain = rules
            .iter()
            .position(|r| r.get("domain").is_some())
            .expect("domain exclusion rule not found");

        assert!(ip < hijack, "ip exclusion must precede the dns hijack");
        assert!(
            domain < hijack,
            "domain exclusion must precede the dns hijack"
        );
    }

    fn rule_index_with_inbound_tag(config: &Value, tag: &str) -> Option<usize> {
        config["routing"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .position(|r| r["inboundTag"] == json!([tag]))
    }

    fn rule_index_with_outbound_tag(config: &Value, tag: &str) -> Option<usize> {
        config["routing"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .position(|r| r["outboundTag"] == json!(tag))
    }

    fn dns_server_with_tag<'a>(config: &'a Value, tag: &str) -> Option<&'a Value> {
        dns_servers_with_tag(config, tag).into_iter().next()
    }

    fn dns_servers_with_tag<'a>(config: &'a Value, tag: &str) -> Vec<&'a Value> {
        config["dns"]["servers"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["tag"] == json!(tag))
            .collect()
    }

    #[test]
    fn test_xray_tun_exclusion_dns() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.exclude_domains = vec!["example.com".to_string()];
        settings.dns.enabled = true;

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let servers = config["dns"]["servers"].as_array().unwrap();
        assert!(
            servers.iter().any(|s| s["domains"]
                .as_array()
                .is_some_and(|d| d.contains(&json!("example.com")))),
            "no server answers the excluded domain"
        );
    }

    #[test]
    fn test_excluded_domains_bind_to_the_direct_detoured_server() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.exclude_domains = vec!["corp.example".to_string()];
        settings.dns.enabled = true;
        settings.dns.use_custom_rules = true;
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

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let servers = config["dns"]["servers"].as_array().unwrap();
        let answering = servers
            .iter()
            .find(|s| {
                s["domains"]
                    .as_array()
                    .is_some_and(|d| d.contains(&json!("corp.example")))
            })
            .expect("no server answers the excluded domain");
        assert_eq!(answering["address"], "77.88.8.8");
        assert_eq!(answering["tag"], DNS_DIRECT_TAG);
        assert!(
            !servers.iter().any(|s| s["address"] == "localhost"),
            "the OS resolver must not be reached for when a direct server exists"
        );
    }

    #[test]
    fn test_proxy_detour_is_the_default_route() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "remote".into(),
            protocol: DnsProtocol::Doh,
            address: "9.9.9.9".into(),
            port: None,
            detour: Some("proxy".into()),
        }];

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        assert!(
            !dns_servers_with_tag(&config, DNS_DIRECT_TAG)
                .iter()
                .any(|s| s["address"] == "https://9.9.9.9/dns-query"),
            "a proxy detour is the default route, not a direct one"
        );
    }

    #[test]
    fn test_xray_no_exclusion_when_tun_disabled() {
        let mut settings = default_settings();
        settings.tun.exclude_routes = vec!["104.16.0.0/13".to_string()];
        settings.tun.exclude_domains = vec!["example.com".to_string()];

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let rules = config["routing"]["rules"].as_array().unwrap();
        for rule in rules {
            assert!(rule.get("ip").is_none());
            assert!(rule.get("domain").is_none());
        }
    }

    #[test]
    fn test_v2ray_never_emits_exclusion() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.exclude_routes = vec!["104.16.0.0/13".to_string()];
        settings.tun.exclude_domains = vec!["example.com".to_string()];

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::V2ray);

        let rules = config["routing"]["rules"].as_array().unwrap();
        for rule in rules {
            assert!(rule.get("ip").is_none());
            assert!(rule.get("domain").is_none());
        }
    }

    #[test]
    fn test_xray_tun_derives_dns_plane_when_dns_disabled() {
        let mut settings = default_settings();
        settings.tun.enabled = true;

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        assert_eq!(config["dns"]["tag"], "dns-internal");
        let servers = config["dns"]["servers"].as_array().unwrap();
        assert!(servers.contains(&json!("https://1.1.1.1/dns-query")));
        assert!(servers.contains(&json!("https://8.8.8.8/dns-query")));
        assert_eq!(config["dns"]["queryStrategy"], "UseIPv4");

        let rules = config["routing"]["rules"].as_array().unwrap();
        let proxy_tag = config["outbounds"][0]["tag"].as_str().unwrap();
        let internal = rule_index_with_inbound_tag(&config, "dns-internal").unwrap();
        assert_eq!(rules[internal]["outboundTag"], proxy_tag);
    }

    #[test]
    fn test_xray_tun_user_dns_gains_internal_tag_and_rule() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        assert_eq!(config["dns"]["tag"], "dns-internal");
        assert!(rule_index_with_inbound_tag(&config, "dns-internal").is_some());
    }

    #[test]
    fn test_xray_tun_profile_dns_overrides_disabled_global_dns() {
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

        let config = generate_v2ray_family_config(
            &[vless_node()],
            &rules,
            &effective,
            V2rayFamilyBackend::Xray,
        );

        assert_eq!(config["dns"]["tag"], "dns-internal");
        assert!(rule_index_with_inbound_tag(&config, "dns-internal").is_some());
    }

    #[test]
    fn test_xray_tun_hijack_emits_dns_out_and_udp53_rule() {
        let mut settings = default_settings();
        settings.tun.enabled = true;

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let outbounds = config["outbounds"].as_array().unwrap();
        assert!(
            outbounds
                .iter()
                .any(|o| o["protocol"] == "dns" && o["tag"] == "dns-out")
        );

        let rules = config["routing"]["rules"].as_array().unwrap();
        let hijack = rule_index_with_outbound_tag(&config, "dns-out").unwrap();
        assert_eq!(rules[hijack]["network"], "tcp,udp");
        assert_eq!(rules[hijack]["port"], 53);
    }

    #[test]
    fn test_xray_tun_native_and_disabled_skip_hijack() {
        for mode in [DnsHijackMode::Native, DnsHijackMode::Disabled] {
            let mut settings = default_settings();
            settings.tun.enabled = true;
            settings.tun.dns_hijack = mode;

            let config = generate_v2ray_family_config(
                &[ss_node()],
                &[],
                &settings,
                V2rayFamilyBackend::Xray,
            );

            let outbounds = config["outbounds"].as_array().unwrap();
            assert!(!outbounds.iter().any(|o| o["protocol"] == "dns"));
            let rules = config["routing"]["rules"].as_array().unwrap();
            assert!(!rules.iter().any(|r| r["outboundTag"] == "dns-out"));
            assert!(
                rule_index_with_inbound_tag(&config, "dns-internal").is_some(),
                "internal-resolver rule stays regardless of hijack mode"
            );
        }
    }

    #[test]
    fn test_xray_tun_emits_hosts_even_when_dns_disabled() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = false;
        settings.dns.hosts = vec![HostOverride {
            domain: "ss.example.com".into(),
            ip: "203.0.113.9".into(),
        }];

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        assert_eq!(config["dns"]["hosts"]["ss.example.com"], "203.0.113.9");
    }

    #[test]
    fn test_hosts_keep_only_the_family_the_strategy_uses() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.strategy = DnsStrategy::Ipv4Only;
        settings.dns.hosts = vec![
            HostOverride {
                domain: "ss.example.com".into(),
                ip: "203.0.113.9".into(),
            },
            HostOverride {
                domain: "ss.example.com".into(),
                ip: "2001:db8::1".into(),
            },
            HostOverride {
                domain: "v6.example.com".into(),
                ip: "2001:db8::2".into(),
            },
        ];

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let hosts = config["dns"]["hosts"].as_object().unwrap();
        assert_eq!(hosts["ss.example.com"], "203.0.113.9");
        assert!(
            !hosts.contains_key("v6.example.com"),
            "a domain left with no usable address must be dropped, not emitted empty"
        );
    }

    #[test]
    fn test_xray_tun_bootstrap_servers_are_scoped_and_direct_tagged() {
        let mut settings = default_settings();
        settings.tun.enabled = true;

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let bootstrap = dns_servers_with_tag(&config, "dns-direct");
        assert_eq!(bootstrap.len(), 2, "one endpoint per transport");

        // Plain UDP first: a network that blocks DoH on 443 is the common case.
        assert_eq!(bootstrap[0]["address"], BOOTSTRAP_DNS_UDP);
        assert_eq!(bootstrap[1]["address"], FALLBACK_DNS);

        for server in &bootstrap {
            assert_eq!(server["domains"], json!(["full:ss.example.com"]));
            assert_eq!(server["skipFallback"], json!(true));
        }

        // Only the last one ends the query, or the second would never be tried.
        assert!(bootstrap[0]["finalQuery"].is_null());
        assert_eq!(bootstrap[1]["finalQuery"], json!(true));

        let direct = rule_index_with_inbound_tag(&config, "dns-direct").expect("no direct rule");
        assert_eq!(
            config["routing"]["rules"][direct]["outboundTag"], "direct",
            "the bootstrap must leave through the marked direct outbound"
        );
    }

    #[test]
    fn test_dns_direct_rule_precedes_the_internal_and_hijack_rules() {
        let mut settings = default_settings();
        settings.tun.enabled = true;

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let direct = rule_index_with_inbound_tag(&config, "dns-direct").unwrap();
        let internal = rule_index_with_inbound_tag(&config, "dns-internal").unwrap();
        let hijack = rule_index_with_outbound_tag(&config, "dns-out").unwrap();

        assert_eq!(direct, 0);
        assert!(direct < internal);
        assert!(
            direct < hijack,
            "the port-53 hijack carries no inboundTag and would swallow the plain-UDP bootstrap"
        );
    }

    #[test]
    fn test_xray_tun_omits_bootstrap_for_ip_addressed_nodes() {
        let mut settings = default_settings();
        settings.tun.enabled = true;

        let node = ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: "203.0.113.9".into(),
            port: 8388,
            method: "aes-256-gcm".into(),
            password: "secret".into(),
            remark: Some("Literal".into()),
        });

        let config =
            generate_v2ray_family_config(&[node], &[], &settings, V2rayFamilyBackend::Xray);

        assert!(dns_server_with_tag(&config, "dns-direct").is_none());
        assert!(rule_index_with_inbound_tag(&config, "dns-direct").is_none());
    }

    #[test]
    fn test_xray_tun_bootstrap_covers_hostname_addressed_dns_servers() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.dns.servers = vec![
            DnsServerConfig {
                tag: "remote".into(),
                protocol: DnsProtocol::Doh,
                address: "dns.adguard.com".into(),
                port: None,
                detour: None,
            },
            DnsServerConfig {
                tag: "domestic".into(),
                protocol: DnsProtocol::Udp,
                address: "94.140.14.14".into(),
                port: None,
                detour: None,
            },
        ];

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let domains = dns_server_with_tag(&config, "dns-direct").unwrap()["domains"]
            .as_array()
            .unwrap()
            .clone();
        assert!(domains.contains(&json!("full:ss.example.com")));
        assert!(domains.contains(&json!("full:dns.adguard.com")));
        assert!(
            !domains.contains(&json!("full:94.140.14.14")),
            "an IP-literal resolver needs no bootstrap"
        );
    }

    #[test]
    fn test_direct_detour_server_is_tagged_for_xray_tun() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "domestic".into(),
            protocol: DnsProtocol::Udp,
            address: "77.88.8.8".into(),
            port: None,
            detour: Some("direct".into()),
        }];

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let tagged: Vec<&Value> = config["dns"]["servers"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["tag"] == json!("dns-direct"))
            .collect();
        assert!(
            tagged.iter().any(|s| s["address"] == "77.88.8.8"),
            "a direct detour must reach the direct outbound"
        );
    }

    #[test]
    fn test_detour_is_not_expressed_without_tun() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "domestic".into(),
            protocol: DnsProtocol::Udp,
            address: "77.88.8.8".into(),
            port: None,
            detour: Some("direct".into()),
        }];

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        assert!(dns_server_with_tag(&config, "dns-direct").is_none());
    }

    #[test]
    fn test_v2ray_backend_gets_no_bootstrap_or_direct_tag() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.dns.servers = vec![DnsServerConfig {
            tag: "domestic".into(),
            protocol: DnsProtocol::Udp,
            address: "77.88.8.8".into(),
            port: None,
            detour: Some("direct".into()),
        }];

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::V2ray);

        assert!(dns_server_with_tag(&config, "dns-direct").is_none());
        assert!(rule_index_with_inbound_tag(&config, "dns-direct").is_none());
    }

    #[test]
    fn test_xray_tun_freedom_uses_builtin_resolver() {
        let mut settings = default_settings();
        settings.tun.enabled = true;

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::Xray);

        let freedom = config["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["protocol"] == "freedom")
            .unwrap();
        assert_eq!(freedom["settings"]["domainStrategy"], "UseIP");
    }

    #[test]
    fn test_xray_no_dns_hardening_without_tun() {
        let config = generate_v2ray_family_config(
            &[ss_node()],
            &[],
            &default_settings(),
            V2rayFamilyBackend::Xray,
        );

        assert!(config.get("dns").is_none());
        let freedom = config["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["protocol"] == "freedom")
            .unwrap();
        assert!(freedom["settings"].get("domainStrategy").is_none());
        assert!(
            !config["outbounds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["protocol"] == "dns")
        );
    }

    #[test]
    fn test_v2ray_tun_never_derives_dns() {
        let mut settings = default_settings();
        settings.tun.enabled = true;

        let config =
            generate_v2ray_family_config(&[ss_node()], &[], &settings, V2rayFamilyBackend::V2ray);

        assert!(config.get("dns").is_none());
    }
}
