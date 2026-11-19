//! Pure summary of the DNS resolvers a generated backend config will actually
//! use. Mirrors the resolver-emitting logic of the generators without parsing
//! any generated JSON: given the same inputs, `effective_dns` classifies every
//! resolver into an ordered list of entries with address, transport, routing
//! path, provenance and scope.

use super::common::strip_suffix_wildcard;
use crate::config::dns_plan::ResolvedDnsPlan;
use crate::geodata::canonical_rule_set_tag;
use crate::models::{
    AUTO_SPLIT_DOMESTIC_TAG, AUTO_SPLIT_REMOTE_TAG, AppSettings, BackendType, DnsProtocol,
    DnsRuleMatch, RoutingRule, RuleAction, RuleMatch, normalize_domain_suffix,
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
    pub fn as_str(self) -> &'static str {
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
    pub fn as_str(self) -> &'static str {
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
        BackendType::V2ray | BackendType::Xray => {
            v2ray_family_dns(backend, settings, rules, node_hosts)
        }
        BackendType::SingBox => singbox_dns(settings, rules),
    };
    effective.uses_fallback = effective
        .entries
        .iter()
        .any(|e| e.source == DnsSource::Fallback);
    effective.system_resolver = effective.entries.iter().any(|e| e.path == DnsPath::System);
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

    let plan = ResolvedDnsPlan::resolve(settings, backend, node_hosts);
    let mut entries = Vec::new();

    if tun_xray && !plan.bootstrap_domains.is_empty() {
        let scope = DnsScope::Domains(plan.bootstrap_domains.clone());
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

    let user = if settings.dns.enabled {
        user_dns_entries(backend, settings, &plan, rules, tun_xray)
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

fn user_dns_entries(
    backend: BackendType,
    settings: &AppSettings,
    plan: &ResolvedDnsPlan,
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
                            format!(
                                "domain:{}",
                                strip_suffix_wildcard(normalize_domain_suffix(suffix))
                            )
                        }
                        DnsRuleMatch::DomainKeyword { keyword } => format!("keyword:{keyword}"),
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

    for server in &plan.servers {
        let transport = server.protocol;
        let address = if backend == BackendType::SingBox {
            server.address.clone()
        } else {
            server.transport_address()
        };
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
        entries.push(entry(
            address,
            Some(transport),
            path,
            DnsSource::User,
            scope,
        ));
    }

    // Excluded domains are split-horizon names. A domain-scoped target absorbs
    // them; an unrestricted target keeps answering everything else, so the
    // exclusions get their own entry alongside it.
    if tun_xray && !settings.tun.exclude_domains.is_empty() {
        let exclude: Vec<String> = settings
            .tun
            .exclude_domains
            .iter()
            .map(|d| format!("domain:{}", strip_suffix_wildcard(d)))
            .collect();
        let target = plan.split_horizon_server();
        let target_entry = target.map(|server| {
            let transport = server.protocol;
            let address = if backend == BackendType::SingBox {
                server.address.clone()
            } else {
                server.transport_address()
            };
            let path = if server.detours_direct() {
                DnsPath::Direct
            } else {
                DnsPath::Proxy
            };
            (address, transport, path)
        });
        let target_addr = target_entry.as_ref().map(|(address, _, _)| address.clone());
        match entries
            .iter()
            .position(|e| Some(&e.address) == target_addr.as_ref())
        {
            Some(index) => match &mut entries[index].scope {
                DnsScope::Domains(domains) => domains.extend(exclude),
                DnsScope::All => {
                    let dedicated = entry(
                        entries[index].address.clone(),
                        entries[index].transport,
                        entries[index].path,
                        entries[index].source,
                        DnsScope::Domains(exclude),
                    );
                    entries.insert(index + 1, dedicated);
                }
            },
            None => match target_entry {
                Some((address, protocol, path)) => entries.push(entry(
                    address,
                    Some(protocol),
                    path,
                    DnsSource::User,
                    DnsScope::Domains(exclude),
                )),
                None => entries.push(entry(
                    "localhost",
                    None,
                    DnsPath::System,
                    DnsSource::User,
                    DnsScope::Domains(exclude),
                )),
            },
        }
    }

    entries
}

fn collect_routing_domains(
    rules: &[RoutingRule],
    remote: &mut Vec<String>,
    domestic: &mut Vec<String>,
) {
    for rule in rules.iter().filter(|r| r.enabled) {
        let domain = match &rule.match_condition {
            RuleMatch::GeoSite { category } => Some(format!("geosite:{category}")),
            RuleMatch::Domain { pattern } => {
                Some(format!("domain:{}", strip_suffix_wildcard(pattern)))
            }
            RuleMatch::DomainKeyword { keyword } => Some(format!("keyword:{keyword}")),
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

/// xray answers a hosts hit authoritatively, so a domain only earns a static
/// entry when some override for it carries a usable address.
fn static_entries_v2ray(settings: &AppSettings) -> Option<Vec<EffectiveDnsEntry>> {
    if settings.dns.hosts.is_empty() {
        return None;
    }
    let mut entries = Vec::new();
    let mut named: Vec<String> = Vec::new();
    for host in &settings.dns.hosts {
        if !host.matches_strategy(settings.dns.strategy) {
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

    let plan = ResolvedDnsPlan::resolve(settings, BackendType::SingBox, &[]);
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
        let final_tag = plan.first_tag();
        let split_horizon_tag = plan.split_horizon_server().map(|s| s.tag.as_str());
        for server in &plan.servers {
            let path = if server.detour.as_deref().is_none_or(|d| d == "direct") {
                DnsPath::Direct
            } else {
                DnsPath::Proxy
            };
            let scope = if Some(server.tag.as_str()) == final_tag {
                DnsScope::All
            } else {
                server_scope_singbox(settings, rules, &server.tag, split_horizon_tag)
            };
            entries.push(entry(
                server.address.clone(),
                Some(server.protocol),
                path,
                DnsSource::User,
                scope,
            ));
        }

        // Mirrors the generated bootstrap in build_dns: same shared
        // selection predicate, `local` without TUN, direct-dialed DoH under
        // TUN where the OS resolver routes through the tunnel.
        if super::singbox::bootstrap_resolver_selected(settings) {
            if settings.tun.enabled {
                entries.push(entry(
                    FALLBACK_DNS,
                    Some(DnsProtocol::Doh),
                    DnsPath::Direct,
                    DnsSource::Bootstrap,
                    DnsScope::All,
                ));
            } else {
                entries.push(entry(
                    "local",
                    None,
                    DnsPath::System,
                    DnsSource::System,
                    DnsScope::All,
                ));
            }
        }
    }

    if !settings.dns.hosts.is_empty() {
        let mut seen: Vec<String> = Vec::new();
        for host in &settings.dns.hosts {
            if !super::singbox::host_matches_singbox_strategy(host, settings.dns.strategy) {
                continue;
            }
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
    split_horizon_tag: Option<&str>,
) -> DnsScope {
    let mut domains: Vec<String> = Vec::new();
    if settings.tun.enabled && Some(tag) == split_horizon_tag {
        for domain in &settings.tun.exclude_domains {
            domains.push(format!("domain:{}", strip_suffix_wildcard(domain)));
        }
    }
    if settings.dns.use_custom_rules {
        for rule in &settings.dns.rules {
            if rule.server_tag != tag {
                continue;
            }
            match &rule.match_condition {
                DnsRuleMatch::GeoSite { category } => {
                    if let Some(tag) = canonical_rule_set_tag(&format!("geosite-{category}")) {
                        domains.push(tag);
                    }
                }
                DnsRuleMatch::DomainSuffix { suffix } => {
                    domains.push(format!(
                        "domain:{}",
                        strip_suffix_wildcard(normalize_domain_suffix(suffix))
                    ));
                }
                DnsRuleMatch::DomainKeyword { keyword } => {
                    domains.push(format!("keyword:{keyword}"));
                }
                DnsRuleMatch::DomainFull { domain } => {
                    domains.push(format!("full:{domain}"));
                }
            }
        }
    } else {
        let mut remote: Vec<String> = Vec::new();
        let mut domestic: Vec<String> = Vec::new();
        for rule in rules.iter().filter(|r| r.enabled) {
            let domain = match &rule.match_condition {
                RuleMatch::GeoSite { category } => {
                    canonical_rule_set_tag(&format!("geosite-{category}"))
                }
                RuleMatch::Domain { pattern } => {
                    Some(format!("domain:{}", strip_suffix_wildcard(pattern)))
                }
                RuleMatch::DomainKeyword { keyword } => Some(format!("keyword:{keyword}")),
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
        let derived = match tag {
            t if t == AUTO_SPLIT_REMOTE_TAG => &remote,
            t if t == AUTO_SPLIT_DOMESTIC_TAG => &domestic,
            _ => &Vec::new(),
        };
        domains.extend(derived.iter().cloned());
    }

    scope_from(domains)
}

impl EffectiveDnsEntry {
    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn transport(&self) -> Option<DnsProtocol> {
        self.transport
    }

    pub fn path(&self) -> DnsPath {
        self.path
    }

    pub fn source(&self) -> DnsSource {
        self.source
    }

    pub fn scope(&self) -> &DnsScope {
        &self.scope
    }
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

    /// Relabels user-configured entries as profile-sourced. Called by the
    /// connection layer after `resolve_effective_config` applied an imported
    /// provider profile's DNS; app-derived fallback, bootstrap and system
    /// entries keep their provenance.
    pub fn mark_profile(&mut self) {
        for entry in &mut self.entries {
            if entry.source == DnsSource::User {
                entry.source = DnsSource::Profile;
            }
        }
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
    use crate::config::SingboxGenerator;
    use crate::config::test_fixtures::fixtures::{default_settings, vless_node};
    use crate::config::v2ray::{V2rayFamilyBackend, generate_v2ray_family_config};
    use crate::models::{
        ConnectionNodeRef, DnsConfig, DnsRule, DnsRuleMatch, DnsServerConfig, DnsStrategy,
        HostOverride, ImportedProfile, ProxyNode, Subscription,
    };
    use chrono::Utc;
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

    fn scoped_server(
        tag: &str,
        protocol: DnsProtocol,
        address: &str,
        detour: &str,
    ) -> DnsServerConfig {
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
        sub.use_imported_profile = true;
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

        let summary = effective_dns(BackendType::Xray, &settings, &[], &[node.address()]);

        assert_eq!(summary.entries().len(), 2);
        assert_eq!(summary.entries()[0].address, FALLBACK_DNS);
        assert_eq!(summary.entries()[0].transport, Some(DnsProtocol::Doh));
        assert_eq!(summary.entries()[0].path, DnsPath::Proxy);
        assert_eq!(summary.entries()[0].source, DnsSource::Fallback);
        assert_eq!(summary.entries()[0].scope, DnsScope::All);
        assert_eq!(summary.entries()[1].address, FALLBACK_DNS_SECONDARY);
        assert_eq!(summary.entries()[1].source, DnsSource::Fallback);
        assert!(
            !summary
                .entries()
                .iter()
                .any(|e| e.address == "1.0.0.1" || e.address.starts_with("https://1.0.0.1"))
        );
        assert!(summary.uses_fallback());
        assert!(!summary.system_resolver());
    }

    #[test]
    fn xray_tun_dns_disabled_with_ip_node_has_no_bootstrap() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.dns.enabled = false;
        let node = node_with_address("203.0.113.10");

        let summary = effective_dns(BackendType::Xray, &settings, &[], &[node.address()]);

        assert!(
            summary
                .entries()
                .iter()
                .all(|e| e.source != DnsSource::Bootstrap)
        );
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
        assert!(
            summary
                .entries()
                .iter()
                .any(|e| e.source == DnsSource::User && e.scope == DnsScope::All)
        );
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
        assert_eq!(last.address, "9.9.9.9:53");
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

        assert!(summary.entries().iter().all(|e| e.address != "local"));
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
    fn xray_downgrades_dot_to_doh() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.servers = vec![server("dot", DnsProtocol::Dot, "dns.adguard.com")];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);

        assert_eq!(summary.entries()[0].transport, Some(DnsProtocol::Doh));
        assert_eq!(
            summary.entries()[0].address,
            "https://dns.adguard.com/dns-query"
        );
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
        settings.dns.rules = vec![dns_rule("google.com", "remote"), dns_rule("ru", "domestic")];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);

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
            .find(|e| e.address == "77.88.8.8:53")
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
            .find(|e| e.address == "77.88.8.8:53")
            .unwrap();
        assert_eq!(
            domestic.scope,
            DnsScope::Domains(vec!["domain:example.ru".into()])
        );
    }

    #[test]
    fn unrestricted_direct_server_keeps_scope_alongside_exclusions() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.exclude_domains = vec!["corp.example.com".into()];
        settings.dns.enabled = true;
        settings.dns.servers = vec![scoped_server("main", DnsProtocol::Udp, "9.9.9.9", "direct")];

        let summary = effective_dns(BackendType::Xray, &settings, &[], &["203.0.113.10"]);
        let matching: Vec<&EffectiveDnsEntry> = summary
            .entries()
            .iter()
            .filter(|e| e.address == "9.9.9.9:53")
            .collect();

        assert_eq!(
            matching.iter().filter(|e| e.scope == DnsScope::All).count(),
            1,
            "the unrestricted server must keep answering everything else: {matching:?}"
        );
        assert!(
            matching
                .iter()
                .any(|e| e.scope == DnsScope::Domains(vec!["domain:corp.example.com".into()])),
            "excluded domains need a scoped entry: {matching:?}"
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
            host("nas.local", "192.168.1.3"),
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
        assert!(
            !summary
                .log_lines()
                .iter()
                .any(|l| l.contains("router.local"))
        );
    }

    #[test]
    fn wrong_family_hosts_omitted_to_match_generated_configs() {
        for backend in [BackendType::V2ray, BackendType::Xray, BackendType::SingBox] {
            let mut settings = default_settings();
            settings.dns.enabled = true;
            settings.dns.strategy = DnsStrategy::Ipv4Only;
            settings.dns.servers = vec![server("main", DnsProtocol::Udp, "9.9.9.9")];
            settings.dns.hosts = vec![
                host("v4.example.com", "192.0.2.10"),
                host("v6.example.com", "2001:db8::10"),
            ];
            let nodes = [node_with_address("203.0.113.10")];
            let config = match backend {
                BackendType::SingBox => SingboxGenerator
                    .generate(&nodes, &[], &settings)
                    .expect("sing-box config"),
                BackendType::V2ray => {
                    generate_v2ray_family_config(&nodes, &[], &settings, V2rayFamilyBackend::V2ray)
                }
                BackendType::Xray => {
                    generate_v2ray_family_config(&nodes, &[], &settings, V2rayFamilyBackend::Xray)
                }
            };
            let summary = effective_dns(backend, &settings, &[], &["203.0.113.10"]);
            let static_domains: std::collections::HashSet<String> = summary
                .entries()
                .iter()
                .filter(|e| e.path == DnsPath::Static)
                .map(|e| e.address.clone())
                .collect();
            let expected: std::collections::HashSet<String> =
                ["v4.example.com".to_string()].into_iter().collect();
            assert_eq!(static_domains, expected, "{backend:?}");

            let table = match backend {
                BackendType::SingBox => &config["dns"]["servers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|s| s["type"] == "hosts")
                    .expect("hosts server")["predefined"],
                _ => &config["dns"]["hosts"],
            };
            let table_keys: std::collections::HashSet<String> = table
                .as_object()
                .expect("hosts table object")
                .keys()
                .cloned()
                .collect();
            assert_eq!(table_keys, expected, "{backend:?}");
        }
    }

    #[test]
    fn imported_profile_dns_marked_profile() {
        let profile_dns = DnsConfig {
            enabled: true,
            servers: vec![server("provider", DnsProtocol::Doh, "doh.provider.example")],
            ..Default::default()
        };
        let sub = profile_subscription(profile_dns);
        let settings = default_settings();

        let (rules, effective_settings) = crate::models::resolve_effective_config(
            &node_ref(&sub),
            std::slice::from_ref(&sub),
            &[],
            &settings,
        );
        assert!(crate::models::uses_imported_profile(
            &node_ref(&sub),
            std::slice::from_ref(&sub)
        ));

        let mut summary = effective_dns(
            BackendType::Xray,
            &effective_settings,
            &rules,
            &["203.0.113.10"],
        );
        // The connection layer applies this when the profile DNS was used.
        summary.mark_profile();

        let user = summary
            .entries()
            .iter()
            .find(|e| e.address == "https://doh.provider.example/dns-query")
            .unwrap();
        assert_eq!(user.source, DnsSource::Profile);
        assert!(
            summary
                .entries()
                .iter()
                .all(|e| e.source != DnsSource::User)
        );
    }

    #[test]
    fn mark_profile_leaves_fallback_bootstrap_and_system() {
        let mut summary = EffectiveDns {
            entries: vec![
                entry(
                    "9.9.9.9",
                    Some(DnsProtocol::Udp),
                    DnsPath::Direct,
                    DnsSource::Bootstrap,
                    DnsScope::All,
                ),
                entry(
                    FALLBACK_DNS,
                    Some(DnsProtocol::Doh),
                    DnsPath::Proxy,
                    DnsSource::Fallback,
                    DnsScope::All,
                ),
                entry(
                    "localhost",
                    None,
                    DnsPath::System,
                    DnsSource::System,
                    DnsScope::All,
                ),
                entry(
                    "1.1.1.1",
                    Some(DnsProtocol::Udp),
                    DnsPath::Routing,
                    DnsSource::User,
                    DnsScope::All,
                ),
            ],
            uses_fallback: true,
            system_resolver: true,
        };
        summary.mark_profile();

        assert_eq!(summary.entries()[0].source, DnsSource::Bootstrap);
        assert_eq!(summary.entries()[1].source, DnsSource::Fallback);
        assert_eq!(summary.entries()[2].source, DnsSource::System);
        assert_eq!(summary.entries()[3].source, DnsSource::Profile);
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

        assert!(lines.contains(
            &"server=1.1.1.1 path=direct source=bootstrap scope=example.com".to_string()
        ));
        assert!(lines.contains(&format!(
            "server={FALLBACK_DNS} path=direct source=bootstrap scope=example.com"
        )));
        assert!(
            lines.contains(
                &"server=77.88.8.8:53 path=direct source=user scope=domain:ru".to_string()
            )
        );
        assert!(lines.iter().any(|l| l.contains("source=fallback")));
        assert!(
            lines
                .iter()
                .all(|l| !l.contains("pinned.local") && !l.contains("source=static"))
        );
    }

    /// The summary stores sing-box addresses raw (`1.1.1.1`) while the derived
    /// TUN resolver and the v2ray family use URL forms for DoH
    /// (`https://1.1.1.1/dns-query`); the host is the common denominator both
    /// sides share for the same logical resolver.
    fn canonical_address(address: &str) -> String {
        if let Some(rest) = address.strip_prefix("udp://") {
            return rest.strip_suffix(":53").unwrap_or(rest).to_string();
        }
        for scheme in ["https://", "h3://", "tls://", "quic://", "tcp://"] {
            if let Some(rest) = address.strip_prefix(scheme) {
                return rest.split(['/', ':']).next().unwrap_or(rest).to_string();
            }
        }
        address.strip_suffix(":53").unwrap_or(address).to_string()
    }

    /// (address set, has-system-resolver) as the generated config carries them.
    /// `system` is true for a missing dns section or an OS-resolver server
    /// (`localhost` for the v2ray family, a `local`-typed server for sing-box).
    fn generated_dns_addresses(
        config: &serde_json::Value,
        backend: BackendType,
    ) -> (std::collections::HashSet<String>, bool) {
        let Some(dns) = config.get("dns") else {
            return (Default::default(), true);
        };
        let mut addresses = std::collections::HashSet::new();
        let mut system = false;
        for server in dns["servers"].as_array().expect("dns.servers array") {
            match backend {
                BackendType::V2ray | BackendType::Xray => {
                    let mut address = server.as_str().map(str::to_string).unwrap_or_else(|| {
                        server["address"]
                            .as_str()
                            .expect("server address")
                            .to_string()
                    });
                    if let Some(port) = server.get("port").and_then(|p| p.as_u64()) {
                        address = format!("{address}:{port}");
                    }
                    if address == "localhost" {
                        system = true;
                    }
                    addresses.insert(canonical_address(&address));
                }
                BackendType::SingBox => match server["type"].as_str().unwrap_or_default() {
                    "fakeip" | "hosts" => {}
                    "local" => {
                        system = true;
                        addresses.insert("local".to_string());
                    }
                    _ => {
                        addresses.insert(canonical_address(
                            server["server"].as_str().expect("typed server address"),
                        ));
                    }
                },
            }
        }
        (addresses, system)
    }

    fn summary_addresses(summary: &EffectiveDns) -> std::collections::HashSet<String> {
        summary
            .entries()
            .iter()
            // Static hosts entries are table overrides, not resolvers; the
            // `system` placeholder stands for an absent dns section, which
            // contributes no generated server.
            .filter(|e| {
                e.path != DnsPath::Static && !(e.path == DnsPath::System && e.address == "system")
            })
            .map(|e| canonical_address(&e.address))
            .collect()
    }

    fn assert_cross_check(
        config: &serde_json::Value,
        summary: &EffectiveDns,
        backend: BackendType,
        label: &str,
    ) {
        let (generated, generated_system) = generated_dns_addresses(config, backend);
        assert_eq!(
            summary_addresses(summary),
            generated,
            "{label}: summary addresses diverge from generated dns.servers"
        );
        assert_eq!(
            summary.system_resolver(),
            generated_system,
            "{label}: system_resolver diverges from generated config"
        );
    }

    fn matrix_settings(tun: bool, dns_enabled: bool, server_mode: &str) -> AppSettings {
        let mut settings = default_settings();
        settings.tun.enabled = tun;
        settings.dns.enabled = dns_enabled;
        if dns_enabled {
            match server_mode {
                "unscoped" => {
                    settings.dns.servers = vec![server("main", DnsProtocol::Udp, "9.9.9.9")];
                }
                "proxy-detoured" => {
                    let mut remote = server("remote", DnsProtocol::Doh, "1.1.1.1");
                    remote.detour = Some("proxy".to_string());
                    settings.dns.servers = vec![remote];
                }
                "custom-rules" => {
                    settings.dns.use_custom_rules = true;
                    settings.dns.servers = vec![
                        server("remote", DnsProtocol::Doh, "1.1.1.1"),
                        server("domestic", DnsProtocol::Udp, "77.88.8.8"),
                    ];
                    settings.dns.rules =
                        vec![dns_rule("google.com", "remote"), dns_rule("ru", "domestic")];
                }
                "auto-split" => {
                    settings.dns.servers = vec![
                        server("remote", DnsProtocol::Doh, "1.1.1.1"),
                        server("domestic", DnsProtocol::Udp, "77.88.8.8"),
                    ];
                }
                _ => unreachable!(),
            }
        } else {
            // Must be ignored: no configured server may reach the summary or
            // the generated config while DNS is disabled.
            settings.dns.servers = vec![server("main", DnsProtocol::Udp, "9.9.9.9")];
        }
        settings
    }

    #[test]
    fn effective_dns_addresses_match_generated_servers() {
        let node_kinds = ["203.0.113.10", "example.com"];
        let server_modes = ["unscoped", "proxy-detoured", "custom-rules", "auto-split"];
        let mut cases = 0;

        for tun in [false, true] {
            for dns_enabled in [false, true] {
                let modes: &[&str] = if dns_enabled {
                    &server_modes
                } else {
                    &["unscoped"]
                };
                for server_mode in modes {
                    for node_addr in node_kinds {
                        let settings = matrix_settings(tun, dns_enabled, server_mode);
                        let rules: Vec<RoutingRule> = if dns_enabled && *server_mode == "auto-split"
                        {
                            vec![
                                RoutingRule {
                                    id: Uuid::new_v4(),
                                    match_condition: RuleMatch::Domain {
                                        pattern: "google.com".into(),
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
                            ]
                        } else {
                            Vec::new()
                        };
                        let label = format!("{tun}/{dns_enabled}/{server_mode}/{node_addr}");

                        for (backend, family) in [
                            (BackendType::V2ray, V2rayFamilyBackend::V2ray),
                            (BackendType::Xray, V2rayFamilyBackend::Xray),
                        ] {
                            let config = generate_v2ray_family_config(
                                &[node_with_address(node_addr)],
                                &rules,
                                &settings,
                                family,
                            );
                            let summary = effective_dns(backend, &settings, &rules, &[node_addr]);
                            assert_cross_check(&config, &summary, backend, &label);
                        }

                        let config = SingboxGenerator
                            .generate(&[node_with_address(node_addr)], &rules, &settings)
                            .expect("sing-box config");
                        let summary =
                            effective_dns(BackendType::SingBox, &settings, &rules, &[node_addr]);
                        assert_cross_check(&config, &summary, BackendType::SingBox, &label);

                        // 60-case budget counts one cell per backend.
                        cases += 3;
                    }
                }
            }
        }

        assert_eq!(cases, 60, "matrix must cover 60 cells");
    }

    /// The summary's bootstrap entry must mirror the generated resolver:
    /// `local`/system resolver without TUN, direct-dialed DoH reported as
    /// source `bootstrap` under TUN, scope `all` in both cases.
    #[test]
    fn singbox_bootstrap_summary_entry_matches_generated_resolver() {
        let cases = [
            (false, "local", None, DnsPath::System, DnsSource::System),
            (
                true,
                FALLBACK_DNS,
                Some(DnsProtocol::Doh),
                DnsPath::Direct,
                DnsSource::Bootstrap,
            ),
        ];
        for (tun, address, transport, path, source) in cases {
            let mut settings = default_settings();
            settings.tun.enabled = tun;
            settings.dns.enabled = true;
            let mut remote = server("remote", DnsProtocol::Doh, "1.1.1.1");
            remote.detour = Some("proxy".to_string());
            settings.dns.servers = vec![remote];

            let summary = effective_dns(BackendType::SingBox, &settings, &[], &["203.0.113.10"]);
            let bootstrap = summary
                .entries()
                .iter()
                .find(|e| e.source == source && e.path == path)
                .expect("bootstrap resolver entry missing from the summary");
            assert_eq!(bootstrap.address, address);
            assert_eq!(bootstrap.transport, transport);
            assert_eq!(bootstrap.scope, DnsScope::All);
            if tun {
                assert!(
                    summary
                        .entries()
                        .iter()
                        .all(|e| !(e.path == DnsPath::System && e.address == "local")),
                    "no local system resolver may be reported under TUN"
                );
            }
        }
    }

    #[test]
    fn hosts_keys_equal_static_entry_scopes_and_stay_out_of_servers() {
        for backend in [BackendType::V2ray, BackendType::Xray, BackendType::SingBox] {
            let mut settings = default_settings();
            settings.dns.enabled = true;
            settings.dns.servers = vec![server("main", DnsProtocol::Udp, "9.9.9.9")];
            settings.dns.hosts = vec![
                host("router.local", "192.168.1.1"),
                host("nas.local", "192.168.1.2"),
            ];
            let nodes = [node_with_address("203.0.113.10")];

            let config = match backend {
                BackendType::SingBox => SingboxGenerator
                    .generate(&nodes, &[], &settings)
                    .expect("sing-box config"),
                BackendType::V2ray => {
                    generate_v2ray_family_config(&nodes, &[], &settings, V2rayFamilyBackend::V2ray)
                }
                BackendType::Xray => {
                    generate_v2ray_family_config(&nodes, &[], &settings, V2rayFamilyBackend::Xray)
                }
            };
            let summary = effective_dns(backend, &settings, &[], &["203.0.113.10"]);

            let static_domains: std::collections::HashSet<String> = summary
                .entries()
                .iter()
                .filter(|e| e.path == DnsPath::Static)
                .map(|e| e.address.clone())
                .collect();
            let expected: std::collections::HashSet<String> = ["router.local", "nas.local"]
                .into_iter()
                .map(str::to_string)
                .collect();
            assert_eq!(static_domains, expected, "{backend:?}");

            // The generated hosts table carries exactly those keys.
            let table = match backend {
                BackendType::SingBox => &config["dns"]["servers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|s| s["type"] == "hosts")
                    .expect("hosts server")["predefined"],
                _ => &config["dns"]["hosts"],
            };
            let table_keys: std::collections::HashSet<String> = table
                .as_object()
                .expect("hosts table object")
                .keys()
                .cloned()
                .collect();
            assert_eq!(table_keys, expected, "{backend:?}");

            // Static entries are not resolvers: absent from log lines and from
            // the generated server address set, which still matches exactly.
            assert!(
                summary
                    .log_lines()
                    .iter()
                    .all(|l| { !l.contains("router.local") && !l.contains("nas.local") })
            );
            assert_cross_check(&config, &summary, backend, "hosts cell");
        }
    }

    #[test]
    fn singbox_custom_and_auto_derived_scopes_preserve_matcher_prefixes() {
        let mut custom = default_settings();
        custom.dns.enabled = true;
        custom.dns.use_custom_rules = true;
        custom.dns.servers = vec![
            scoped_server("remote", DnsProtocol::Doh, "1.1.1.1", "proxy"),
            scoped_server("domestic", DnsProtocol::Udp, "77.88.8.8", "direct"),
        ];
        custom.dns.rules = vec![
            DnsRule {
                match_condition: DnsRuleMatch::GeoSite {
                    category: "category-ru".into(),
                },
                server_tag: "domestic".into(),
            },
            DnsRule {
                match_condition: DnsRuleMatch::DomainSuffix {
                    suffix: "*.example.ru".into(),
                },
                server_tag: "domestic".into(),
            },
            DnsRule {
                match_condition: DnsRuleMatch::DomainKeyword {
                    keyword: "yandex".into(),
                },
                server_tag: "domestic".into(),
            },
            DnsRule {
                match_condition: DnsRuleMatch::DomainFull {
                    domain: "api.example.ru".into(),
                },
                server_tag: "domestic".into(),
            },
        ];

        let summary = effective_dns(BackendType::SingBox, &custom, &[], &["203.0.113.10"]);
        let domestic = summary
            .entries()
            .iter()
            .find(|e| e.address == "77.88.8.8")
            .unwrap();
        assert_eq!(
            domestic.scope,
            DnsScope::Domains(vec![
                "geosite-category-ru".into(),
                "domain:example.ru".into(),
                "keyword:yandex".into(),
                "full:api.example.ru".into(),
            ])
        );

        let mut auto = default_settings();
        auto.dns.enabled = true;
        auto.dns.servers = custom.dns.servers.clone();
        let routing_rules = vec![
            RoutingRule {
                id: Uuid::new_v4(),
                match_condition: RuleMatch::GeoSite {
                    category: "category-ru".into(),
                },
                action: RuleAction::Direct,
                enabled: true,
                group: None,
                via_node: None,
            },
            RoutingRule {
                id: Uuid::new_v4(),
                match_condition: RuleMatch::Domain {
                    pattern: "*.example.ru".into(),
                },
                action: RuleAction::Direct,
                enabled: true,
                group: None,
                via_node: None,
            },
            RoutingRule {
                id: Uuid::new_v4(),
                match_condition: RuleMatch::DomainKeyword {
                    keyword: "yandex".into(),
                },
                action: RuleAction::Direct,
                enabled: true,
                group: None,
                via_node: None,
            },
            RoutingRule {
                id: Uuid::new_v4(),
                match_condition: RuleMatch::DomainFull {
                    domain: "api.example.ru".into(),
                },
                action: RuleAction::Direct,
                enabled: true,
                group: None,
                via_node: None,
            },
        ];

        let auto_summary = effective_dns(
            BackendType::SingBox,
            &auto,
            &routing_rules,
            &["203.0.113.10"],
        );
        let auto_domestic = auto_summary
            .entries()
            .iter()
            .find(|e| e.address == "77.88.8.8")
            .unwrap();
        assert_eq!(
            auto_domestic.scope,
            DnsScope::Domains(vec![
                "geosite-category-ru".into(),
                "domain:example.ru".into(),
                "keyword:yandex".into(),
                "full:api.example.ru".into(),
            ])
        );
    }

    #[test]
    fn singbox_unrepresentable_geosite_tag_omitted_from_scope() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.use_custom_rules = true;
        settings.dns.servers = vec![
            scoped_server("remote", DnsProtocol::Doh, "1.1.1.1", "proxy"),
            scoped_server("domestic", DnsProtocol::Udp, "77.88.8.8", "direct"),
        ];
        settings.dns.rules = vec![
            DnsRule {
                match_condition: DnsRuleMatch::GeoSite {
                    category: "geolocation-!cn".into(),
                },
                server_tag: "domestic".into(),
            },
            dns_rule("cn", "domestic"),
        ];

        let summary = effective_dns(BackendType::SingBox, &settings, &[], &["203.0.113.10"]);
        let domestic = summary
            .entries()
            .iter()
            .find(|e| e.address == "77.88.8.8")
            .unwrap();
        assert_eq!(domestic.scope, DnsScope::Domains(vec!["domain:cn".into()]));
    }

    #[test]
    fn singbox_tun_excluded_domains_appear_in_non_final_split_horizon_scope() {
        let mut settings = default_settings();
        settings.tun.enabled = true;
        settings.tun.exclude_domains = vec!["*.corp.example.com".into()];
        settings.dns.enabled = true;
        settings.dns.servers = vec![
            scoped_server("remote", DnsProtocol::Doh, "1.1.1.1", "proxy"),
            scoped_server("domestic", DnsProtocol::Udp, "77.88.8.8", "direct"),
        ];

        let summary = effective_dns(BackendType::SingBox, &settings, &[], &["203.0.113.10"]);
        let remote = summary
            .entries()
            .iter()
            .find(|e| e.address == "1.1.1.1")
            .unwrap();
        assert_eq!(remote.scope, DnsScope::All);
        let domestic = summary
            .entries()
            .iter()
            .find(|e| e.address == "77.88.8.8")
            .unwrap();
        assert_eq!(
            domestic.scope,
            DnsScope::Domains(vec!["domain:corp.example.com".into()])
        );
    }

    #[test]
    fn v2ray_and_xray_keyword_scope_is_prefixed() {
        let mut settings = default_settings();
        settings.dns.enabled = true;
        settings.dns.use_custom_rules = true;
        settings.dns.servers = vec![server("domestic", DnsProtocol::Udp, "77.88.8.8")];
        settings.dns.rules = vec![DnsRule {
            match_condition: DnsRuleMatch::DomainKeyword {
                keyword: "yandex".into(),
            },
            server_tag: "domestic".into(),
        }];

        for backend in [BackendType::V2ray, BackendType::Xray] {
            let summary = effective_dns(backend, &settings, &[], &["203.0.113.10"]);
            let domestic = summary
                .entries()
                .iter()
                .find(|e| e.address == "77.88.8.8:53")
                .unwrap();
            assert_eq!(
                domestic.scope,
                DnsScope::Domains(vec!["keyword:yandex".into()]),
                "{backend:?}"
            );
        }
    }
}
