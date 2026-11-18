use crate::models::{
    AppSettings, BackendType, DnsProtocol, DnsServerConfig, DnsStrategy, HostOverride,
};

/// A configured resolver lowered for one backend: the protocol the binary can
/// actually speak, the port that protocol implies, and the detour flag the
/// generators branch on.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDnsServer {
    pub tag: String,
    pub address: String,
    pub protocol: DnsProtocol,
    pub requested_protocol: DnsProtocol,
    pub port: Option<u16>,
    pub detour: Option<String>,
    pub resolves_private: bool,
}

impl ResolvedDnsServer {
    pub fn port_or_default(&self) -> u16 {
        self.port.unwrap_or_else(|| self.protocol.default_port())
    }

    pub fn detours_direct(&self) -> bool {
        self.detour.as_deref() == Some("direct")
    }

    /// A downgraded protocol cannot honour the configured port because the URL
    /// shape changes with it, so the fallback protocol's default port applies.
    pub fn transport_address(&self) -> String {
        let port = if self.protocol != self.requested_protocol {
            None
        } else {
            self.port
        };
        self.protocol.server_address(&self.address, port)
    }

    pub fn v2ray_address(&self) -> String {
        let address = self.transport_address();
        // Xray and v2ray parse a bare `host:port` UDP address as a URL and
        // reject it; the explicit scheme is what makes the port acceptable.
        if self.protocol == DnsProtocol::Udp {
            format!("udp://{address}")
        } else {
            address
        }
    }
}

/// Everything a backend generator needs from the DNS settings before it starts
/// shaping its own JSON: the effective server list, the names that must resolve
/// before the tunnel can carry traffic, and the host overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDnsPlan {
    pub backend: BackendType,
    pub enabled: bool,
    pub tun_enabled: bool,
    pub strategy: DnsStrategy,
    pub servers: Vec<ResolvedDnsServer>,
    pub bootstrap_domains: Vec<String>,
    pub hosts: Vec<HostOverride>,
}

impl ResolvedDnsPlan {
    pub fn resolve(settings: &AppSettings, backend: BackendType, node_hosts: &[&str]) -> Self {
        let servers = settings
            .dns
            .servers
            .iter()
            .map(|server| resolve_server(server, backend, settings.tun.enabled))
            .collect();

        Self {
            backend,
            enabled: settings.dns.enabled,
            tun_enabled: settings.tun.enabled,
            strategy: settings.dns.strategy,
            servers,
            bootstrap_domains: bootstrap_domains(settings, node_hosts),
            hosts: settings.dns.hosts.clone(),
        }
    }

    pub fn first_tag(&self) -> Option<&str> {
        self.servers.first().map(|server| server.tag.as_str())
    }

    pub fn warn_compat(&self) {
        for server in &self.servers {
            if server.protocol != server.requested_protocol {
                log::warn!(
                    "{} does not support {:?} DNS for '{}' directly; falling back to {:?}",
                    backend_name(self.backend),
                    server.requested_protocol,
                    server.tag,
                    server.protocol
                );
            }
            if server.resolves_private {
                log::warn!(
                    "DNS server {} ({}) is private and routed through the proxy; its queries reach the proxy server's network - consider detour 'direct'",
                    server.tag,
                    server.address
                );
            }
        }
    }

    pub fn server(&self, tag: &str) -> Option<&ResolvedDnsServer> {
        self.servers.iter().find(|server| server.tag == tag)
    }

    pub fn split_horizon_server(&self) -> Option<&ResolvedDnsServer> {
        self.servers
            .iter()
            .find(|server| server.detours_direct())
            .or_else(|| self.servers.first())
    }

    /// xray and v2ray only accept resolver domains through `full:` entries.
    pub fn full_bootstrap_domains(&self) -> Vec<String> {
        self.bootstrap_domains
            .iter()
            .map(|domain| format!("full:{domain}"))
            .collect()
    }
}

fn resolve_server(
    server: &DnsServerConfig,
    backend: BackendType,
    tun_enabled: bool,
) -> ResolvedDnsServer {
    let protocol = server.protocol.effective_for_backend(backend);

    ResolvedDnsServer {
        tag: server.tag.clone(),
        address: server.address.clone(),
        protocol,
        requested_protocol: server.protocol,
        port: server.port,
        detour: server.detour.clone(),
        resolves_private: server.resolves_via_proxy_private(backend, tun_enabled),
    }
}

fn backend_name(backend: BackendType) -> &'static str {
    match backend {
        BackendType::V2ray => "v2ray",
        BackendType::Xray => "xray",
        BackendType::SingBox => "sing-box",
    }
}

fn bootstrap_domains(settings: &AppSettings, node_hosts: &[&str]) -> Vec<String> {
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
    if !domains.iter().any(|domain| domain == host) {
        domains.push(host.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(tag: &str, protocol: DnsProtocol, address: &str) -> DnsServerConfig {
        DnsServerConfig {
            tag: tag.to_string(),
            protocol,
            address: address.to_string(),
            port: None,
            detour: None,
        }
    }

    #[test]
    fn v2ray_udp_addresses_carry_the_explicit_scheme() {
        let plan = ResolvedDnsPlan {
            backend: BackendType::Xray,
            enabled: true,
            tun_enabled: false,
            strategy: DnsStrategy::default(),
            servers: vec![resolve_server(
                &server("udp", DnsProtocol::Udp, "9.9.9.9"),
                BackendType::Xray,
                false,
            )],
            bootstrap_domains: Vec::new(),
            hosts: Vec::new(),
        };

        assert_eq!(plan.servers[0].v2ray_address(), "udp://9.9.9.9:53");
        assert_eq!(plan.servers[0].transport_address(), "9.9.9.9:53");
    }

    #[test]
    fn unsupported_protocols_fall_back_and_drop_the_custom_port() {
        let mut config = server("h3", DnsProtocol::H3, "dns.example.com");
        config.port = Some(8443);

        let resolved = resolve_server(&config, BackendType::V2ray, false);

        assert_eq!(resolved.requested_protocol, DnsProtocol::H3);
        assert_eq!(resolved.protocol, DnsProtocol::Doh);
        assert_eq!(
            resolved.v2ray_address(),
            "https://dns.example.com/dns-query"
        );
    }

    #[test]
    fn singbox_keeps_the_requested_protocol() {
        let resolved = resolve_server(
            &server("doq", DnsProtocol::Doq, "dns.example.com"),
            BackendType::SingBox,
            false,
        );

        assert_eq!(resolved.protocol, DnsProtocol::Doq);
        assert_eq!(resolved.port_or_default(), 853);
    }

    #[test]
    fn bootstrap_domains_skip_addresses_and_deduplicate() {
        let mut settings = AppSettings::default();
        settings.dns.enabled = true;
        settings.dns.servers = vec![
            server("doh", DnsProtocol::Doh, "dns.example.com"),
            server("literal", DnsProtocol::Udp, "1.1.1.1"),
        ];

        let plan = ResolvedDnsPlan::resolve(
            &settings,
            BackendType::Xray,
            &["proxy.example.com", "192.0.2.1", "proxy.example.com"],
        );

        assert_eq!(
            plan.bootstrap_domains,
            ["proxy.example.com", "dns.example.com"]
        );
        assert_eq!(
            plan.full_bootstrap_domains(),
            ["full:proxy.example.com", "full:dns.example.com"]
        );
    }

    #[test]
    fn split_horizon_prefers_the_direct_detour() {
        let mut settings = AppSettings::default();
        let mut direct = server("domestic", DnsProtocol::Udp, "192.0.2.53");
        direct.detour = Some("direct".into());
        settings.dns.servers = vec![server("remote", DnsProtocol::Doh, "1.1.1.1"), direct];

        let plan = ResolvedDnsPlan::resolve(&settings, BackendType::SingBox, &[]);

        assert_eq!(
            plan.split_horizon_server()
                .map(|server| server.tag.as_str()),
            Some("domestic")
        );
    }
}
