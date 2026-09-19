//! Pure summary of the DNS resolvers a generated backend config will actually
//! use. Mirrors the resolver-emitting logic of the generators without parsing
//! any generated JSON: given the same inputs, `effective_dns` classifies every
//! resolver into an ordered list of entries with address, transport, routing
//! path, provenance and scope.

use crate::models::{
    AUTO_SPLIT_DOMESTIC_TAG, AUTO_SPLIT_REMOTE_TAG, AppSettings, BackendType, DnsProtocol,
    DnsRuleMatch, RoutingRule, RuleAction, RuleMatch,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveDns {
    entries: Vec<EffectiveDnsEntry>,
    uses_fallback: bool,
    system_resolver: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveDnsEntry {
    address: String,
    transport: Option<DnsProtocol>,
    path: DnsPath,
    source: DnsSource,
    scope: DnsScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsPath {
    /// Leaves through the `direct` outbound (explicit detour, or a bootstrap
    /// resolver that must not recurse through the proxy being resolved).
    Direct,
    /// Routed through the proxy chain (xray TUN `dns-internal`, sing-box
    /// `detour`).
    Proxy,
    /// Dispatched by the routing rule set; outside TUN unmatched traffic takes
    /// the first outbound, so the resolver follows the routing config.
    Routing,
    /// The operating system resolver (`localhost` / sing-box `local`), or no
    /// DNS section at all.
    System,
    /// Authoritative hosts-table override, not a network resolver.
    Static,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsSource {
    /// Configured by the user in `settings.dns.servers`.
    User,
    /// Appended by a generator as resolver of last resort.
    Fallback,
    /// Prepended to resolve names the tunnel itself depends on.
    Bootstrap,
    /// The operating system resolver.
    System,
    /// A user server whose values came from an imported provider profile.
    Profile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnsScope {
    All,
    Domains(Vec<String>),
}

impl DnsPath {
    fn as_str(self) -> &'static str {
        match self {
            DnsPath::Direct => "direct",
            DnsPath::Proxy => "proxy",
            DnsPath::Routing => "routing",
            DnsPath::System => "system",
            DnsPath::Static => "static",
        }
    }
}

impl DnsSource {
    fn as_str(self) -> &'static str {
        match self {
            DnsSource::User => "user",
            DnsSource::Fallback => "fallback",
            DnsSource::Bootstrap => "bootstrap",
            DnsSource::System => "system",
            DnsSource::Profile => "profile",
        }
    }
}

impl std::fmt::Display for DnsScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DnsScope::All => write!(f, "all"),
            DnsScope::Domains(domains) => write!(f, "{}", domains.join(",")),
        }
    }
}

fn entry(
    address: impl Into<String>,
    transport: Option<DnsProtocol>,
    path: DnsPath,
    source: DnsSource,
    scope: DnsScope,
) -> EffectiveDnsEntry {
    EffectiveDnsEntry {
        address: address.into(),
        transport,
        path,
        source,
        scope,
    }
}

const FALLBACK_DNS: &str = "https://1.1.1.1/dns-query";
const FALLBACK_DNS_SECONDARY: &str = "https://8.8.8.8/dns-query";
const BOOTSTRAP_DNS_UDP: &str = "1.1.1.1";

/// Classifies every resolver the generated config for `backend` will carry,
/// in generation order. `node_hosts` are the proxy nodes' addresses; server
/// addresses are taken from `settings` directly. Pure: no IO, no parsing.
pub fn effective_dns(
    backend: BackendType,
    settings: &AppSettings,
    rules: &[RoutingRule],
    node_hosts: &[&str],
) -> EffectiveDns {
    let mut effective = match backend {
        BackendType::V2ray | BackendType::Xray => v2ray_family_dns(backend, settings, rules, node_hosts),
        BackendType::SingBox => singbox_dns(settings, rules),
    };
    effective.uses_fallback = effective
        .entries
        .iter()
        .any(|e| e.source == DnsSource::Fallback);
    effective.system_resolver = effective
        .entries
        .iter()
        .any(|e| e.path == DnsPath::System);
    effective
}

fn no_dns_section() -> EffectiveDns {
    EffectiveDns {
        entries: vec![entry(
            "system",
            None,
            DnsPath::System,
            DnsSource::System,
            DnsScope::All,
        )],
        uses_fallback: false,
        system_resolver: true,
    }
}

fn v2ray_family_dns(
    backend: BackendType,
    settings: &AppSettings,
    rules: &[RoutingRule],
    node_hosts: &[&str],
) -> EffectiveDns {
    let tun_xray = backend == BackendType::Xray && settings.tun.enabled;

    if !(settings.dns.enabled || tun_xray) {
        return no_dns_section();
    }

    let mut entries = Vec::new();

    if tun_xray {
        let domains = bootstrap_domains(node_hosts, settings);
        if !domains.is_empty() {
            let scope = DnsScope::Domains(domains);
            entries.push(entry(
                BOOTSTRAP_DNS_UDP,
                Some(DnsProtocol::Udp),
                DnsPath::Direct,
                DnsSource::Bootstrap,
                scope.clone(),
            ));
            entries.push(entry(
                FALLBACK_DNS,
                Some(DnsProtocol::Doh),
                DnsPath::Direct,
                DnsSource::Bootstrap,
                scope,
            ));
        }
    }

    let user = if settings.dns.enabled {
        user_dns_entries(backend, settings, rules, tun_xray)
    } else {
        vec![
            entry(
                FALLBACK_DNS,
                Some(DnsProtocol::Doh),
                DnsPath::Proxy,
                DnsSource::Fallback,
                DnsScope::All,
            ),
            entry(
                FALLBACK_DNS_SECONDARY,
                Some(DnsProtocol::Doh),
                DnsPath::Proxy,
                DnsSource::Fallback,
                DnsScope::All,
            ),
        ]
    };
    entries.extend(user);

    if settings.dns.enabled && entries.iter().all(|e| e.source != DnsSource::User) {
        entries.push(if tun_xray {
            entry(
                FALLBACK_DNS,
                Some(DnsProtocol::Doh),
                DnsPath::Proxy,
                DnsSource::Fallback,
                DnsScope::All,
            )
        } else {
            entry(
                "localhost",
                None,
                DnsPath::System,
                DnsSource::System,
                DnsScope::All,
            )
        });
    }

    // xray marks every scoped server skipFallback and makes sure something
    // unrestricted remains to answer the rest. The `localhost` entry for an
    // empty server list counts as unrestricted, like any plain-string server
    // in the generated array.
    if backend == BackendType::Xray {
        let unrestricted = entries
            .iter()
            .filter(|e| !matches!(e.source, DnsSource::Bootstrap | DnsSource::Profile))
            .any(|e| e.scope == DnsScope::All);
        if !unrestricted {
            entries.push(entry(
                FALLBACK_DNS,
                Some(DnsProtocol::Doh),
                if tun_xray {
                    DnsPath::Proxy
                } else {
                    DnsPath::Routing
                },
                DnsSource::Fallback,
                DnsScope::All,
            ));
        }
    }

    if let Some(statics) = static_entries_v2ray(settings) {
        entries.extend(statics);
    }

    EffectiveDns {
        entries,
        uses_fallback: false,
        system_resolver: false,
    }
}

fn bootstrap_domains(node_hosts: &[&str], settings: &AppSettings) -> Vec<String> {
    let mut domains: Vec<String> = Vec::new();
    for host in node_hosts {
        push_bootstrap_domain(&mut domains, host);
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
    if !domains.iter().any(|d| d == host) {
        domains.push(host.to_string());
    }
}

fn user_dns_entries(
    backend: BackendType,
    settings: &AppSettings,
    rules: &[RoutingRule],
    tun_xray: bool,
) -> Vec<EffectiveDnsEntry> {
    let mut entries: Vec<EffectiveDnsEntry> = Vec::new();

    let scopes: Vec<(String, DnsScope)> = if settings.dns.use_custom_rules {
        settings
            .dns
            .servers
            .iter()
            .map(|server| {
                let domains: Vec<String> = settings
                    .dns
                    .rules
                    .iter()
                    .filter(|rule| rule.server_tag == server.tag)
                    .map(|rule| match &rule.match_condition {
                        DnsRuleMatch::GeoSite { category } => format!("geosite:{category}"),
                        DnsRuleMatch::DomainSuffix { suffix } => {
                            format!("domain:{}", strip_suffix_wildcard(suffix))
                        }
                        DnsRuleMatch::DomainKeyword { keyword } => keyword.clone(),
                        DnsRuleMatch::DomainFull { domain } => format!("full:{domain}"),
                    })
                    .collect();
                (server.tag.clone(), scope_from(domains))
            })
            .collect()
    } else {
        let mut remote: Vec<String> = Vec::new();
        let mut domestic: Vec<String> = Vec::new();
        collect_routing_domains(rules, &mut remote, &mut domestic);
        if tun_xray && !settings.tun.exclude_domains.is_empty() {
            for d in &settings.tun.exclude_domains {
                domestic.push(format!("domain:{}", strip_suffix_wildcard(d)));
            }
        }
        settings
            .dns
            .servers
            .iter()
            .map(|server| {
                let domains = match server.tag.as_str() {
                    t if t == AUTO_SPLIT_REMOTE_TAG => remote.clone(),
                    t if t == AUTO_SPLIT_DOMESTIC_TAG => domestic.clone(),
                    _ => Vec::new(),
                };
                (server.tag.clone(), scope_from(domains))
            })
            .collect()
    };

    for server in &settings.dns.servers {
        let transport = server.protocol.effective_for_backend(backend);
        let address = transport.server_address(
            &server.address,
            if transport != server.protocol {
                None
            } else {
                server.port
            },
        );
        let scope = scopes
            .iter()
            .find(|(tag, _)| *tag == server.tag)
            .map(|(_, scope)| scope.clone())
            .unwrap_or(DnsScope::All);
        let path = if tun_xray && server.detours_direct() {
            DnsPath::Direct
        } else if tun_xray {
            DnsPath::Proxy
        } else {
            DnsPath::Routing
        };
        entries.push(entry(address, Some(transport), path, DnsSource::User, scope));
    }

    // Excluded domains are split-horizon names: folded into the server that
    // resolves outside the tunnel when there is one, OS resolver otherwise.
    if tun_xray
        && settings.dns.use_custom_rules
        && !settings.tun.exclude_domains.is_empty()
    {
        let exclude: Vec<String> = settings
            .tun
            .exclude_domains
            .iter()
            .map(|d| format!("domain:{}", strip_suffix_wildcard(d)))
            .collect();
        let target = settings
            .dns
            .servers
            .iter()
            .find(|s| s.detours_direct())
            .or_else(|| settings.dns.servers.first());
        let target_addr = target.map(|s| {
            s.protocol
                .effective_for_backend(backend)
                .server_address(&s.address, s.port)
        });
        match entries
            .iter_mut()
            .find(|e| Some(&e.address) == target_addr.as_ref())
        {
            Some(matched) => match &mut matched.scope {
                DnsScope::Domains(domains) => domains.extend(exclude),
                DnsScope::All => {}
            },
            None => entries.push(entry(
                "localhost",
                None,
                DnsPath::System,
                DnsSource::User,
                DnsScope::Domains(exclude),
            )),
        }
    }

    entries
}

fn collect_routing_domains(rules: &[RoutingRule], remote: &mut Vec<String>, domestic: &mut Vec<String>) {
    for rule in rules.iter().filter(|r| r.enabled) {
        let domain = match &rule.match_condition {
            RuleMatch::GeoSite { category } => Some(format!("geosite:{category}")),
            RuleMatch::Domain { pattern } => {
                Some(format!("domain:{}", strip_suffix_wildcard(pattern)))
            }
            RuleMatch::DomainKeyword { keyword } => Some(keyword.clone()),
            RuleMatch::DomainFull { domain } => Some(format!("full:{domain}")),
            _ => None,
        };
        if let Some(domain) = domain {
            match rule.action {
                RuleAction::Proxy => remote.push(domain),
                RuleAction::Direct => domestic.push(domain),
                RuleAction::Block => {}
            }
        }
    }
}

fn scope_from(domains: Vec<String>) -> DnsScope {
    if domains.is_empty() {
        DnsScope::All
    } else {
        DnsScope::Domains(domains)
    }
}

fn strip_suffix_wildcard(pattern: &str) -> &str {
    pattern.strip_prefix("*.").unwrap_or(pattern)
}

/// xray answers a hosts hit authoritatively, so a domain only earns a static
/// entry when some override for it carries a usable address.
fn static_entries_v2ray(settings: &AppSettings) -> Option<Vec<EffectiveDnsEntry>> {
    if settings.dns.hosts.is_empty() {
        return None;
    }
    let mut entries = Vec::new();
    let mut named: Vec<String> = Vec::new();
    for host in &settings.dns.hosts {
        if host.ip.parse::<std::net::IpAddr>().is_err() {
            continue;
        }
        if !named.contains(&host.domain) {
            named.push(host.domain.clone());
            entries.push(entry(
                host.domain.clone(),
                None,
                DnsPath::Static,
                DnsSource::User,
                DnsScope::Domains(vec![host.domain.clone()]),
            ));
        }
    }
    (!entries.is_empty()).then_some(entries)
}

fn singbox_dns(settings: &AppSettings, rules: &[RoutingRule]) -> EffectiveDns {
    if !(settings.dns.enabled || settings.tun.enabled) {
        return no_dns_section();
    }

    let mut entries = Vec::new();

    if !settings.dns.enabled {
        // Derived TUN plane: a single DoH resolver detoured through the proxy.
        entries.push(entry(
            FALLBACK_DNS,
            Some(DnsProtocol::Doh),
            DnsPath::Proxy,
            DnsSource::Fallback,
            DnsScope::All,
        ));
    } else {
        let final_tag = settings.dns.servers.first().map(|s| s.tag.clone());
        for server in &settings.dns.servers {
            let path = if server.detour.as_deref().is_none_or(|d| d == "direct") {
                DnsPath::Direct
            } else {
                DnsPath::Proxy
            };
            let mut scope = server_scope_singbox(settings, rules, &server.tag);
            if Some(&server.tag) == final_tag.as_ref() {
                scope = DnsScope::All;
            }
            entries.push(entry(
                server.address.clone(),
                Some(server.protocol),
                path,
                DnsSource::User,
                scope,
            ));
        }

        if !settings.dns.servers.is_empty()
            && !settings
                .dns
                .servers
                .iter()
                .any(|s| s.address.parse::<std::net::IpAddr>().is_ok())
        {
            entries.push(entry(
                "local",
                None,
                DnsPath::System,
                DnsSource::System,
                DnsScope::All,
            ));
        }
    }

    if !settings.dns.hosts.is_empty() {
        let mut seen: Vec<String> = Vec::new();
        for host in &settings.dns.hosts {
            if !seen.contains(&host.domain) {
                seen.push(host.domain.clone());
                entries.push(entry(
                    host.domain.clone(),
                    None,
                    DnsPath::Static,
                    DnsSource::User,
                    DnsScope::Domains(vec![host.domain.clone()]),
                ));
            }
        }
    }

    EffectiveDns {
        entries,
        uses_fallback: false,
        system_resolver: false,
    }
}

fn server_scope_singbox(
    settings: &AppSettings,
    rules: &[RoutingRule],
    tag: &str,
) -> DnsScope {
    let mut domains: Vec<String> = Vec::new();
    if settings.dns.use_custom_rules {
        for rule in &settings.dns.rules {
            if rule.server_tag != tag {
                continue;
            }
            match &rule.match_condition {
                DnsRuleMatch::GeoSite { category } => domains.push(format!("geosite-{category}")),
                DnsRuleMatch::DomainSuffix { suffix } => {
                    domains.push(strip_suffix_wildcard(suffix).to_string())
                }
                DnsRuleMatch::DomainKeyword { keyword } => domains.push(keyword.clone()),
                DnsRuleMatch::DomainFull { domain } => domains.push(domain.clone()),
            }
        }
    } else {
        let mut remote: Vec<String> = Vec::new();
        let mut domestic: Vec<String> = Vec::new();
        for rule in rules.iter().filter(|r| r.enabled) {
            let domain = match &rule.match_condition {
                RuleMatch::GeoSite { category } => Some(format!("geosite-{category}")),
                RuleMatch::Domain { pattern } => {
                    Some(strip_suffix_wildcard(pattern).to_string())
                }
                RuleMatch::DomainKeyword { keyword } => Some(keyword.clone()),
                RuleMatch::DomainFull { domain } => Some(domain.clone()),
                _ => None,
            };
            if let Some(domain) = domain {
                match rule.action {
                    RuleAction::Proxy => remote.push(domain),
                    RuleAction::Direct => domestic.push(domain),
                    RuleAction::Block => {}
                }
            }
        }
        let derived = match tag {
            t if t == AUTO_SPLIT_REMOTE_TAG => &remote,
            t if t == AUTO_SPLIT_DOMESTIC_TAG => &domestic,
            _ => &Vec::new(),
        };
        domains.extend(derived.iter().cloned());
    }

    // TUN exclusions route to the split-horizon server.
    if settings.tun.enabled
        && super::common::split_horizon_server(settings).is_some_and(|s| s.tag == tag)
    {
        for d in &settings.tun.exclude_domains {
            domains.push(strip_suffix_wildcard(d).to_string());
        }
    }

    scope_from(domains)
}

impl EffectiveDns {
    pub fn entries(&self) -> &[EffectiveDnsEntry] {
        &self.entries
    }

    pub fn uses_fallback(&self) -> bool {
        self.uses_fallback
    }

    pub fn system_resolver(&self) -> bool {
        self.system_resolver
    }

    /// One line per non-static entry, for the connection log.
    pub fn log_lines(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.path != DnsPath::Static)
            .map(|e| {
                format!(
                    "server={} path={} source={} scope={}",
                    e.address,
                    e.path.as_str(),
                    e.source.as_str(),
                    e.scope
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_fixtures::fixtures::{default_settings, vless_node};
    use crate::models::{
        DnsRule, DnsRuleMatch, DnsServerConfig, HostOverride, ProxyNode,
    };
    use uuid::Uuid;

    fn node_with_address(address: &str) -> ProxyNode {
        let ProxyNode::Vless(mut c) = vless_node() else {
            unreachable!()
        };
        c.address = address.to_string();
        ProxyNode::Vless(c)
    }

    fn server(tag: &str, protocol: DnsProtocol, address: &str) -> DnsServerConfig {
        DnsServerConfig {
            tag: tag.to_string(),
            protocol,
            address: address.to_string(),
            port: None,
            detour: None,
        }
    }

    fn scoped_server(tag: &str, protocol: DnsProtocol, address: &str, detour: &str) -> DnsServerConfig {
        DnsServerConfig {
            tag: tag.to_string(),
            protocol,
            address: address.to_string(),
            port: None,
            detour: Some(detour.to_string()),
        }
    }

    fn dns_rule(suffix: &str, server_tag: &str) -> DnsRule {
        DnsRule {
            match_condition: DnsRuleMatch::DomainSuffix {
                suffix: suffix.to_string(),
            },
            server_tag: server_tag.to_string(),
        }
    }

    fn host(domain: &str, ip: &str) -> HostOverride {
        HostOverride {
            domain: domain.to_string(),
            ip: ip.to_string(),
        }
    }

    fn profile_subscription(dns: DnsConfig) -> Subscription {
        let mut sub = Subscription::new_from_url("Provider", "https://example.com/sub");
        sub.imported_profile = Some(ImportedProfile {
            rules: Vec::new(),
            dns: Some(dns),
            skipped: Vec::new(),
            imported_at: Utc::now(),
        });
        sub
    }

    fn node_ref(sub: &Subscription) -> ConnectionNodeRef {
        ConnectionNodeRef::Subscription {
            subscription_id: sub.id,
            node_id: Uuid::new_v4(),
        }
    }

    #[test]
    fn xray_tun_dns_disabled_lists_fallback_pair_not_configured() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = false;
        settings.dns.servers = vec![server("cloud", DnsProtocol::Doh, "1.0.0.1")];
        let node = node_with_address("203.0.113.10");

        let summary = effective_dns(
            BackendType::Xray,
            &settings,
            &[],
            &[node.address()],
        );

        assert_eq!(summary.entries().len(), 2);
        assert_eq!(summary.entries()[0].address, FALLBACK_DNS);
        assert_eq!(summary.entries()[0].transport, Some(DnsProtocol::Doh));
        assert_eq!(summary.entries()[0].path, DnsPath::Proxy);
        assert_eq!(summary.entries()[0].source, DnsSource::Fallback);
        assert_eq!(summary.entries()[0].scope, DnsScope::All);
        assert_eq!(summary.entries()[1].address, FALLBACK_DNS_SECONDARY);
        assert_eq!(summary.entries()[1].source, DnsSource::Fallback);
        assert!(!summary
            .entries()
            .iter()
            .any(|e| e.address == "1.0.0.1" || e.address.starts_with("https://1.0.0.1")));
        assert!(summary.uses_fallback());
        assert!(!summary.system_resolver());
    }

    #[test]
    fn xray_tun_dns_disabled_with_ip_node_has_no_bootstrap() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = false;
        let node = node_with_address("203.0.113.10");

        let summary = effective_dns(
            BackendType::Xray,
            &settings,
            &[],
            &[node.address()],
        );

        assert!(summary
            .entries()
            .iter()
            .all(|e| e.source != DnsSource::Bootstrap));
    }

    #[test]
    fn xray_all_servers_scoped_appends_unscoped_fallback() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.use_custom_rules = true;
        settings.dns.servers = vec![server("domestic", DnsProtocol::Udp, "77.88.8.8")];
        settings.dns.rules = vec![dns_rule("ru", "domestic")];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);

        let last = summary.entries().last().unwrap();
        assert_eq!(last.address, FALLBACK_DNS);
        assert_eq!(last.source, DnsSource::Fallback);
        assert_eq!(last.scope, DnsScope::All);
        assert_eq!(last.path, DnsPath::Routing);
        assert!(summary.uses_fallback());
    }

    #[test]
    fn uses_fallback_false_with_unscoped_user_server() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![server("main", DnsProtocol::Udp, "1.1.1.1")];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);

        assert!(!summary.uses_fallback());
        assert!(summary
            .entries()
            .iter()
            .any(|e| e.source == DnsSource::User && e.scope == DnsScope::All));
    }

    #[test]
    fn xray_tun_hostname_bootstrap_pair_direct_scoped() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.dns.servers = vec![server("main", DnsProtocol::Udp, "9.9.9.9")];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["example.com"]);

        assert_eq!(summary.entries()[0].address, BOOTSTRAP_DNS_UDP);
        assert_eq!(summary.entries()[0].transport, Some(DnsProtocol::Udp));
        assert_eq!(summary.entries()[0].path, DnsPath::Direct);
        assert_eq!(summary.entries()[0].source, DnsSource::Bootstrap);
        assert_eq!(
            summary.entries()[0].scope,
            DnsScope::Domains(vec!["example.com".to_string()])
        );
        assert_eq!(summary.entries()[1].address, FALLBACK_DNS);
        assert_eq!(summary.entries()[1].transport, Some(DnsProtocol::Doh));
        assert_eq!(summary.entries()[1].path, DnsPath::Direct);
        assert_eq!(summary.entries()[1].source, DnsSource::Bootstrap);
        // The unscoped 9.9.9.9 server answers everything else, so xray adds no
        // fallback of its own.
        let last = summary.entries().last().unwrap();
        assert_eq!(last.address, "9.9.9.9");
        assert_eq!(last.source, DnsSource::User);
        assert_eq!(last.path, DnsPath::Proxy);
        assert!(!summary.uses_fallback());
    }

    #[test]
    fn xray_tun_server_hostname_joins_bootstrap_scope() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.dns.servers = vec![server("doh", DnsProtocol::Doh, "dns.google")];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);

        let bootstrap = summary
            .entries()
            .iter()
            .find(|e| e.source == DnsSource::Bootstrap)
            .unwrap();
        assert_eq!(
            bootstrap.scope,
            DnsScope::Domains(vec!["dns.google".to_string()])
        );
    }

    #[test]
    fn singbox_tun_dns_disabled_derived_doh_fallback() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = false;
        settings.dns.servers = vec![server("cloud", DnsProtocol::Doh, "1.0.0.1")];

        let summary = effective_dns(BackendType::SingBox, &settings, &[], &["203.0.113.10"]);

        assert_eq!(summary.entries().len(), 1);
        assert_eq!(summary.entries()[0].address, FALLBACK_DNS);
        assert_eq!(summary.entries()[0].transport, Some(DnsProtocol::Doh));
        assert_eq!(summary.entries()[0].path, DnsPath::Proxy);
        assert_eq!(summary.entries()[0].source, DnsSource::Fallback);
        assert_eq!(summary.entries()[0].scope, DnsScope::All);
        assert!(summary.uses_fallback());
    }

    #[test]
    fn singbox_server_without_detour_is_direct() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![server("main", DnsProtocol::Udp, "1.1.1.1")];

        let summary = effective_dns(BackendType::SingBox, &settings, &[], &["203.0.113.10"]);

        assert_eq!(summary.entries()[0].path, DnsPath::Direct);
        assert_eq!(summary.entries()[0].address, "1.1.1.1");
        assert_eq!(summary.entries()[0].transport, Some(DnsProtocol::Udp));
    }

    #[test]
    fn singbox_server_with_proxy_detour_is_proxy() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![scoped_server("main", DnsProtocol::Doh, "1.1.1.1", "proxy")];

        let summary = effective_dns(BackendType::SingBox, &settings, &[], &["203.0.113.10"]);

        assert_eq!(summary.entries()[0].path, DnsPath::Proxy);
    }

    #[test]
    fn singbox_hostname_only_servers_append_local_bootstrap() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![server("doh", DnsProtocol::Doh, "dns.google")];

        let summary = effective_dns(BackendType::SingBox, &settings, &[], &["203.0.113.10"]);

        let local = summary.entries().last().unwrap();
        assert_eq!(local.address, "local");
        assert_eq!(local.path, DnsPath::System);
        assert_eq!(local.source, DnsSource::System);
        assert!(summary.system_resolver());
    }

    #[test]
    fn singbox_ip_server_needs_no_local_bootstrap() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![server("main", DnsProtocol::Udp, "1.1.1.1")];

        let summary = effective_dns(BackendType::SingBox, &settings, &[], &["203.0.113.10"]);

        assert!(summary
            .entries()
            .iter()
            .all(|e| e.address != "local"));
    }

    #[test]
    fn no_dns_section_outside_tun_single_system_entry() {
        for backend in [BackendType::V2ray, BackendType::Xray, BackendType::SingBox] {
            let settings = default_settings();
            let summary = effective_dns(backend, &settings, &[], &["203.0.113.10"]);

            assert_eq!(summary.entries().len(), 1, "{backend:?}");
            assert_eq!(summary.entries()[0].address, "system");
            assert_eq!(summary.entries()[0].transport, None);
            assert_eq!(summary.entries()[0].path, DnsPath::System);
            assert_eq!(summary.entries()[0].source, DnsSource::System);
            assert_eq!(summary.entries()[0].scope, DnsScope::All);
            assert!(summary.system_resolver());
            assert!(!summary.uses_fallback());
        }
    }

    #[test]
    fn v2ray_enabled_empty_servers_falls_back_to_localhost() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = Vec::new();

        for backend in [BackendType::V2ray, BackendType::Xray] {
            let summary = effective_dns(backend, &settings, &[], &["203.0.113.10"]);
            assert_eq!(summary.entries().len(), 1, "{backend:?}");
            assert_eq!(summary.entries()[0].address, "localhost");
            assert_eq!(summary.entries()[0].path, DnsPath::System);
            assert_eq!(summary.entries()[0].source, DnsSource::System);
            assert!(summary.system_resolver());
        }
    }

    #[test]
    fn v2ray_outside_tun_untagged_server_is_routing() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![server("main", DnsProtocol::Udp, "1.1.1.1")];

        for backend in [BackendType::V2ray, BackendType::Xray] {
            let summary = effective_dns(backend, &settings, &[], &["203.0.113.10"]);
            assert_eq!(summary.entries()[0].path, DnsPath::Routing, "{backend:?}");
        }
    }

    #[test]
    fn xray_tun_direct_detour_is_direct_path() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.dns.servers = vec![scoped_server(
            "domestic",
            DnsProtocol::Udp,
            "77.88.8.8",
            "direct",
        )];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);

        let user = summary
            .entries()
            .iter()
            .find(|e| e.source == DnsSource::User)
            .unwrap();
        assert_eq!(user.path, DnsPath::Direct);
    }

    #[test]
    fn xray_tun_untagged_server_is_proxy_path() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.dns.servers = vec![server("main", DnsProtocol::Udp, "1.1.1.1")];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);

        let user = summary
            .entries()
            .iter()
            .find(|e| e.source == DnsSource::User)
            .unwrap();
        assert_eq!(user.path, DnsPath::Proxy);
    }

    #[test]
    fn v2ray_downgrades_unsupported_protocols_to_doh() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![server("dot", DnsProtocol::Dot, "dns.adguard.com")];

        let summary = effective_dns(BackendType::V2ray, &settings, &[], &["203.0.113.10"]);

        assert_eq!(summary.entries()[0].transport, Some(DnsProtocol::Doh));
        assert_eq!(
            summary.entries()[0].address,
            "https://dns.adguard.com/dns-query"
        );
    }

    #[test]
    fn xray_keeps_supported_transport_and_port() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![server("dot", DnsProtocol::Dot, "dns.adguard.com")];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);

        assert_eq!(summary.entries()[0].transport, Some(DnsProtocol::Dot));
        assert_eq!(summary.entries()[0].address, "tls://dns.adguard.com");
    }

    #[test]
    fn custom_rules_scope_servers_by_dns_rules() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.use_custom_rules = true;
        settings.dns.servers = vec![
            server("remote", DnsProtocol::Doh, "1.1.1.1"),
            server("domestic", DnsProtocol::Udp, "77.88.8.8"),
        ];
        settings.dns.rules = vec![
            dns_rule("google.com", "remote"),
            dns_rule("ru", "domestic"),
        ];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);

        let remote = summary
            .entries()
            .iter()
            .find(|e| e.address == "https://1.1.1.1/dns-query")
            .unwrap();
        assert_eq!(remote.scope, DnsScope::Domains(vec!["domain:google.com".into()]));
        let domestic = summary
            .entries()
            .iter()
            .find(|e| e.address == "77.88.8.8")
            .unwrap();
        assert_eq!(domestic.scope, DnsScope::Domains(vec!["domain:ru".into()]));
    }

    #[test]
    fn auto_split_derives_scope_from_routing_rules() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![
            server("remote", DnsProtocol::Doh, "1.1.1.1"),
            server("domestic", DnsProtocol::Udp, "77.88.8.8"),
        ];
        let rules = vec![
            RoutingRule {
                id: Uuid::new_v4(),
                match_condition: RuleMatch::Domain {
                    pattern: "*.google.com".into(),
                },
                action: RuleAction::Proxy,
                enabled: true,
                group: None,
                via_node: None,
            },
            RoutingRule {
                id: Uuid::new_v4(),
                match_condition: RuleMatch::Domain {
                    pattern: "example.ru".into(),
                },
                action: RuleAction::Direct,
                enabled: true,
                group: None,
                via_node: None,
            },
            RoutingRule {
                id: Uuid::new_v4(),
                match_condition: RuleMatch::Domain {
                    pattern: "ads.example".into(),
                },
                action: RuleAction::Block,
                enabled: true,
                group: None,
                via_node: None,
            },
        ];

        let summary = effective_dns(BackendType::Xray, &settings, &rules, &["203.0.113.10"]);

        let remote = summary
            .entries()
            .iter()
            .find(|e| e.address == "https://1.1.1.1/dns-query")
            .unwrap();
        assert_eq!(
            remote.scope,
            DnsScope::Domains(vec!["domain:google.com".into()])
        );
        let domestic = summary
            .entries()
            .iter()
            .find(|e| e.address == "77.88.8.8")
            .unwrap();
        assert_eq!(
            domestic.scope,
            DnsScope::Domains(vec!["domain:example.ru".into()])
        );
    }

    #[test]
    fn hosts_overrides_are_static_entries() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![server("main", DnsProtocol::Udp, "1.1.1.1")];
        settings.dns.hosts = vec![
            host("router.local", "192.168.1.1"),
            host("router.local", "192.168.1.2"),
            host("nas.local", "fd00::10"),
        ];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);

        let statics: Vec<&EffectiveDnsEntry> = summary
            .entries()
            .iter()
            .filter(|e| e.path == DnsPath::Static)
            .collect();
        assert_eq!(statics.len(), 2);
        assert_eq!(statics[0].address, "router.local");
        assert_eq!(statics[0].source, DnsSource::User);
        assert_eq!(
            statics[0].scope,
            DnsScope::Domains(vec!["router.local".into()])
        );
        assert_eq!(statics[1].address, "nas.local");
        assert!(!summary.log_lines().iter().any(|l| l.contains("router.local")));
    }

    #[test]
    fn log_lines_format_is_server_path_source_scope() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = true;
        settings.dns.servers = vec![scoped_server(
            "domestic",
            DnsProtocol::Udp,
            "77.88.8.8",
            "direct",
        )];
        settings.dns.use_custom_rules = true;
        settings.dns.rules = vec![dns_rule("ru", "domestic")];
        settings.dns.hosts = vec![host("pinned.local", "10.0.0.1")];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["example.com"]);
        let lines = summary.log_lines();

        assert!(lines.contains(&format!(
            "server=1.1.1.1 path=direct source=bootstrap scope=example.com"
        )));
        assert!(lines.contains(&format!(
            "server={FALLBACK_DNS} path=direct source=bootstrap scope=example.com"
        )));
        assert!(lines.contains(&"server=77.88.8.8 path=direct source=user scope=domain:ru".to_string()));
        assert!(lines.iter().any(|l| l.contains("source=fallback")));
        assert!(lines
            .iter()
            .all(|l| !l.contains("pinned.local") && !l.contains("source=static")));
    }
}
