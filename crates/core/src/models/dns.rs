use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DnsProtocol {
    Udp,
    Tcp,
    Doh,
    Dot,
    Doq,
    H3,
}

impl DnsProtocol {
    pub fn server_address(&self, address: &str, port: Option<u16>) -> String {
        let default_port = match self {
            DnsProtocol::Udp | DnsProtocol::Tcp => 53,
            DnsProtocol::Doh | DnsProtocol::H3 => 443,
            DnsProtocol::Dot | DnsProtocol::Doq => 853,
        };

        match self {
            DnsProtocol::Udp => {
                if port == Some(default_port) || port.is_none() {
                    format!("{}:{}", address, default_port)
                } else {
                    format!("{}:{}", address, port.unwrap())
                }
            }
            DnsProtocol::Tcp => {
                if port == Some(default_port) || port.is_none() {
                    format!("tcp://{}:{}", address, default_port)
                } else {
                    format!("tcp://{}:{}", address, port.unwrap())
                }
            }
            DnsProtocol::Doh => {
                if port == Some(default_port) || port.is_none() {
                    format!("https://{}/dns-query", address)
                } else {
                    format!("https://{}:{}/dns-query", address, port.unwrap())
                }
            }
            DnsProtocol::Dot => {
                if port == Some(default_port) || port.is_none() {
                    format!("tls://{}", address)
                } else {
                    format!("tls://{}:{}", address, port.unwrap())
                }
            }
            DnsProtocol::Doq => {
                if port == Some(default_port) || port.is_none() {
                    format!("quic://{}", address)
                } else {
                    format!("quic://{}:{}", address, port.unwrap())
                }
            }
            DnsProtocol::H3 => {
                if port == Some(default_port) || port.is_none() {
                    format!("h3://{}/dns-query", address)
                } else {
                    format!("h3://{}:{}/dns-query", address, port.unwrap())
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DnsServerConfig {
    pub tag: String,
    pub protocol: DnsProtocol,
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detour: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsStrategy {
    #[default]
    PreferIpv4,
    PreferIpv6,
    Ipv4Only,
    Ipv6Only,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DnsRuleMatch {
    GeoSite { category: String },
    DomainSuffix { suffix: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DnsRule {
    pub match_condition: DnsRuleMatch,
    pub server_tag: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FakeIpConfig {
    #[serde(default = "default_fakeip_enabled")]
    pub enabled: bool,
    #[serde(default = "default_fakeip_inet4")]
    pub inet4_range: String,
    #[serde(default = "default_fakeip_inet6")]
    pub inet6_range: String,
}

fn default_fakeip_enabled() -> bool {
    false
}

fn default_fakeip_inet4() -> String {
    "198.18.0.0/15".into()
}

fn default_fakeip_inet6() -> String {
    "fc00::/18".into()
}

impl Default for FakeIpConfig {
    fn default() -> Self {
        Self {
            enabled: default_fakeip_enabled(),
            inet4_range: default_fakeip_inet4(),
            inet6_range: default_fakeip_inet6(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostOverride {
    pub domain: String,
    pub ip: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "DnsConfigWire")]
pub struct DnsConfig {
    pub enabled: bool,
    #[serde(default)]
    pub strategy: DnsStrategy,
    pub servers: Vec<DnsServerConfig>,
    #[serde(default)]
    pub rules: Vec<DnsRule>,
    #[serde(default)]
    pub fakeip: FakeIpConfig,
    #[serde(default)]
    pub disable_cache: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_subnet: Option<String>,
    #[serde(default)]
    pub hosts: Vec<HostOverride>,
    #[serde(default)]
    pub use_custom_rules: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct DnsConfigWire {
    enabled: bool,
    #[serde(default)]
    strategy: DnsStrategy,
    #[serde(default)]
    servers: Vec<DnsServerConfig>,
    remote: Option<LegacyDnsServer>,
    domestic: Option<LegacyDnsServer>,
    #[serde(default)]
    rules: Vec<DnsRule>,
    #[serde(default)]
    fakeip: FakeIpConfig,
    #[serde(default)]
    disable_cache: bool,
    #[serde(default)]
    client_subnet: Option<String>,
    #[serde(default)]
    hosts: Vec<HostOverride>,
    #[serde(default)]
    use_custom_rules: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyDnsServer {
    protocol: LegacyDnsProtocol,
    address: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LegacyDnsProtocol {
    Plain,
    DoH,
}

impl From<DnsConfigWire> for DnsConfig {
    fn from(wire: DnsConfigWire) -> Self {
        let servers = if !wire.servers.is_empty() {
            wire.servers
        } else if let (Some(remote), Some(domestic)) = (wire.remote, wire.domestic) {
            vec![
                migrate_legacy_server("remote", remote),
                migrate_legacy_server("domestic", domestic),
            ]
        } else {
            DnsConfig::default().servers
        };

        Self {
            enabled: wire.enabled,
            strategy: wire.strategy,
            servers,
            rules: wire.rules,
            fakeip: wire.fakeip,
            disable_cache: wire.disable_cache,
            client_subnet: wire.client_subnet,
            hosts: wire.hosts,
            use_custom_rules: wire.use_custom_rules,
        }
    }
}

fn migrate_legacy_server(tag: &str, legacy: LegacyDnsServer) -> DnsServerConfig {
    DnsServerConfig {
        tag: tag.to_string(),
        protocol: match legacy.protocol {
            LegacyDnsProtocol::Plain => DnsProtocol::Udp,
            LegacyDnsProtocol::DoH => DnsProtocol::Doh,
        },
        address: legacy.address,
        port: None,
        detour: None,
    }
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            strategy: DnsStrategy::default(),
            servers: vec![
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
            ],
            rules: Vec::new(),
            fakeip: FakeIpConfig::default(),
            disable_cache: false,
            client_subnet: None,
            hosts: Vec::new(),
            use_custom_rules: false,
        }
    }
}

impl DnsConfig {
    pub fn validate(&self) -> Result<(), DnsValidationError> {
        let mut tags = HashSet::new();
        for server in &self.servers {
            if !tags.insert(&server.tag) {
                return Err(DnsValidationError::DuplicateServerTag(server.tag.clone()));
            }
        }

        if let Some(ref subnet) = self.client_subnet {
            if subnet.parse::<std::net::IpAddr>().is_err() {
                return Err(DnsValidationError::InvalidClientSubnet(subnet.clone()));
            }
        }

        for rule in &self.rules {
            if !tags.contains(&rule.server_tag) {
                return Err(DnsValidationError::InvalidRuleServerTag(
                    rule.server_tag.clone(),
                ));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum DnsValidationError {
    #[error("duplicate server tag: {0}")]
    DuplicateServerTag(String),
    #[error("invalid client_subnet IP: {0}")]
    InvalidClientSubnet(String),
    #[error("DNS rule references non-existent server tag: {0}")]
    InvalidRuleServerTag(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dns_protocol_udp_default_port() {
        let addr = DnsProtocol::Udp.server_address("8.8.8.8", None);
        assert_eq!(addr, "8.8.8.8:53");
    }

    #[test]
    fn test_dns_protocol_udp_custom_port() {
        let addr = DnsProtocol::Udp.server_address("8.8.8.8", Some(5353));
        assert_eq!(addr, "8.8.8.8:5353");
    }

    #[test]
    fn test_dns_protocol_udp_default_port_explicit() {
        let addr = DnsProtocol::Udp.server_address("8.8.8.8", Some(53));
        assert_eq!(addr, "8.8.8.8:53");
    }

    #[test]
    fn test_dns_protocol_tcp_default_port() {
        let addr = DnsProtocol::Tcp.server_address("8.8.8.8", None);
        assert_eq!(addr, "tcp://8.8.8.8:53");
    }

    #[test]
    fn test_dns_protocol_doh_default_port() {
        let addr = DnsProtocol::Doh.server_address("1.1.1.1", None);
        assert_eq!(addr, "https://1.1.1.1/dns-query");
    }

    #[test]
    fn test_dns_protocol_doh_custom_port() {
        let addr = DnsProtocol::Doh.server_address("1.1.1.1", Some(8443));
        assert_eq!(addr, "https://1.1.1.1:8443/dns-query");
    }

    #[test]
    fn test_dns_protocol_dot_default_port() {
        let addr = DnsProtocol::Dot.server_address("1.1.1.1", None);
        assert_eq!(addr, "tls://1.1.1.1");
    }

    #[test]
    fn test_dns_protocol_dot_custom_port() {
        let addr = DnsProtocol::Dot.server_address("1.1.1.1", Some(8530));
        assert_eq!(addr, "tls://1.1.1.1:8530");
    }

    #[test]
    fn test_dns_protocol_doq_default_port() {
        let addr = DnsProtocol::Doq.server_address("dns.adguard.com", None);
        assert_eq!(addr, "quic://dns.adguard.com");
    }

    #[test]
    fn test_dns_protocol_doq_custom_port() {
        let addr = DnsProtocol::Doq.server_address("dns.adguard.com", Some(784));
        assert_eq!(addr, "quic://dns.adguard.com:784");
    }

    #[test]
    fn test_dns_protocol_h3_default_port() {
        let addr = DnsProtocol::H3.server_address("dns.google", None);
        assert_eq!(addr, "h3://dns.google/dns-query");
    }

    #[test]
    fn test_dns_protocol_h3_custom_port() {
        let addr = DnsProtocol::H3.server_address("dns.google", Some(1443));
        assert_eq!(addr, "h3://dns.google:1443/dns-query");
    }

    #[test]
    fn test_dns_config_default() {
        let cfg = DnsConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.strategy, DnsStrategy::PreferIpv4);
        assert_eq!(cfg.servers.len(), 2);
        assert_eq!(cfg.servers[0].tag, "remote");
        assert_eq!(cfg.servers[0].protocol, DnsProtocol::Doh);
        assert_eq!(cfg.servers[0].address, "1.1.1.1");
        assert_eq!(cfg.servers[1].tag, "domestic");
        assert_eq!(cfg.servers[1].protocol, DnsProtocol::Udp);
        assert_eq!(cfg.servers[1].address, "223.5.5.5");
        assert!(cfg.rules.is_empty());
        assert!(!cfg.fakeip.enabled);
        assert!(!cfg.disable_cache);
        assert!(cfg.client_subnet.is_none());
        assert!(cfg.hosts.is_empty());
        assert!(!cfg.use_custom_rules);
    }

    #[test]
    fn test_dns_config_roundtrip() {
        let cfg = DnsConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: DnsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn test_dns_config_roundtrip_with_all_fields() {
        let cfg = DnsConfig {
            enabled: true,
            strategy: DnsStrategy::Ipv6Only,
            servers: vec![DnsServerConfig {
                tag: "cloudflare".to_string(),
                protocol: DnsProtocol::Doq,
                address: "dns.adguard.com".to_string(),
                port: Some(784),
                detour: Some("proxy-out".to_string()),
            }],
            rules: vec![DnsRule {
                match_condition: DnsRuleMatch::GeoSite {
                    category: "cn".to_string(),
                },
                server_tag: "cloudflare".to_string(),
            }],
            fakeip: FakeIpConfig {
                enabled: true,
                inet4_range: "198.18.0.0/16".to_string(),
                inet6_range: "fc00::/16".to_string(),
            },
            disable_cache: true,
            client_subnet: Some("203.0.113.1".to_string()),
            hosts: vec![HostOverride {
                domain: "example.com".to_string(),
                ip: "192.0.2.1".to_string(),
            }],
            use_custom_rules: true,
        };

        let json = serde_json::to_string(&cfg).unwrap();
        let back: DnsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn test_backward_compat_migration_from_legacy() {
        let legacy_toml = r#"
enabled = true
remote = { protocol = "doh", address = "1.1.1.1" }
domestic = { protocol = "plain", address = "223.5.5.5" }
"#;

        let cfg: DnsConfig = toml::from_str(legacy_toml).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.servers.len(), 2);
        assert_eq!(cfg.servers[0].tag, "remote");
        assert_eq!(cfg.servers[0].protocol, DnsProtocol::Doh);
        assert_eq!(cfg.servers[0].address, "1.1.1.1");
        assert_eq!(cfg.servers[1].tag, "domestic");
        assert_eq!(cfg.servers[1].protocol, DnsProtocol::Udp);
        assert_eq!(cfg.servers[1].address, "223.5.5.5");
    }

    #[test]
    fn test_new_format_direct_load() {
        let new_toml = r#"
enabled = true
strategy = "ipv4_only"

[[servers]]
tag = "cloudflare"
protocol = "doh"
address = "1.1.1.1"

[[servers]]
tag = "google"
protocol = "doh"
address = "8.8.8.8"

[[rules]]
match_condition = { type = "geo_site", category = "cn" }
server_tag = "cloudflare"

[[rules]]
match_condition = { type = "domain_suffix", suffix = ".google.com" }
server_tag = "google"
"#;

        let cfg: DnsConfig = toml::from_str(new_toml).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.strategy, DnsStrategy::Ipv4Only);
        assert_eq!(cfg.servers.len(), 2);
        assert_eq!(cfg.servers[0].tag, "cloudflare");
        assert_eq!(cfg.servers[0].protocol, DnsProtocol::Doh);
        assert_eq!(cfg.servers[1].tag, "google");
        assert_eq!(cfg.servers.len(), 2);
        assert_eq!(cfg.rules.len(), 2);
        assert!(matches!(
            &cfg.rules[0].match_condition,
            DnsRuleMatch::GeoSite { category } if category == "cn"
        ));
        assert_eq!(cfg.rules[0].server_tag, "cloudflare");
        assert!(matches!(
            &cfg.rules[1].match_condition,
            DnsRuleMatch::DomainSuffix { suffix } if suffix == ".google.com"
        ));
        assert_eq!(cfg.rules[1].server_tag, "google");
    }

    #[test]
    fn test_validate_duplicate_server_tags() {
        let cfg = DnsConfig {
            enabled: true,
            strategy: DnsStrategy::default(),
            servers: vec![
                DnsServerConfig {
                    tag: "remote".to_string(),
                    protocol: DnsProtocol::Doh,
                    address: "1.1.1.1".to_string(),
                    port: None,
                    detour: None,
                },
                DnsServerConfig {
                    tag: "remote".to_string(),
                    protocol: DnsProtocol::Udp,
                    address: "223.5.5.5".to_string(),
                    port: None,
                    detour: None,
                },
            ],
            rules: Vec::new(),
            fakeip: FakeIpConfig::default(),
            disable_cache: false,
            client_subnet: None,
            hosts: Vec::new(),
            use_custom_rules: false,
        };

        let result = cfg.validate();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DnsValidationError::DuplicateServerTag(_)
        ));
    }

    #[test]
    fn test_validate_invalid_client_subnet() {
        let cfg = DnsConfig {
            enabled: true,
            strategy: DnsStrategy::default(),
            servers: vec![DnsServerConfig {
                tag: "remote".to_string(),
                protocol: DnsProtocol::Doh,
                address: "1.1.1.1".to_string(),
                port: None,
                detour: None,
            }],
            rules: Vec::new(),
            fakeip: FakeIpConfig::default(),
            disable_cache: false,
            client_subnet: Some("not-an-ip".to_string()),
            hosts: Vec::new(),
            use_custom_rules: false,
        };

        let result = cfg.validate();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DnsValidationError::InvalidClientSubnet(_)
        ));
    }

    #[test]
    fn test_validate_invalid_rule_server_tag() {
        let cfg = DnsConfig {
            enabled: true,
            strategy: DnsStrategy::default(),
            servers: vec![DnsServerConfig {
                tag: "remote".to_string(),
                protocol: DnsProtocol::Doh,
                address: "1.1.1.1".to_string(),
                port: None,
                detour: None,
            }],
            rules: vec![DnsRule {
                match_condition: DnsRuleMatch::GeoSite {
                    category: "cn".to_string(),
                },
                server_tag: "nonexistent".to_string(),
            }],
            fakeip: FakeIpConfig::default(),
            disable_cache: false,
            client_subnet: None,
            hosts: Vec::new(),
            use_custom_rules: false,
        };

        let result = cfg.validate();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DnsValidationError::InvalidRuleServerTag(_)
        ));
    }

    #[test]
    fn test_validate_success() {
        let cfg = DnsConfig {
            enabled: true,
            strategy: DnsStrategy::default(),
            servers: vec![
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
            ],
            rules: vec![DnsRule {
                match_condition: DnsRuleMatch::GeoSite {
                    category: "cn".to_string(),
                },
                server_tag: "domestic".to_string(),
            }],
            fakeip: FakeIpConfig::default(),
            disable_cache: false,
            client_subnet: Some("203.0.113.1".to_string()),
            hosts: Vec::new(),
            use_custom_rules: false,
        };

        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_fakeip_config_default() {
        let cfg = FakeIpConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.inet4_range, "198.18.0.0/15");
        assert_eq!(cfg.inet6_range, "fc00::/18");
    }

    #[test]
    fn test_dns_strategy_serialization() {
        let strategies = vec![
            DnsStrategy::PreferIpv4,
            DnsStrategy::PreferIpv6,
            DnsStrategy::Ipv4Only,
            DnsStrategy::Ipv6Only,
        ];

        for strategy in strategies {
            let json = serde_json::to_string(&strategy).unwrap();
            let back: DnsStrategy = serde_json::from_str(&json).unwrap();
            assert_eq!(strategy, back);
        }
    }
}
