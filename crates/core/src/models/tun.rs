use serde::{Deserialize, Serialize};

use super::validation::{
    ValidationError, validate_domain_pattern, validate_ip_cidr, validate_tun_interface_name,
};

/// Network stack used by the TUN inbound. Serializes to the backend literals
/// (`system`, `gvisor`, `mixed`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunStack {
    #[default]
    System,
    Gvisor,
    Mixed,
}

/// How the TUN inbound treats DNS traffic. Serializes to the backend `dns_mode`
/// literals (`hijack`, `native`, `disabled`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DnsHijackMode {
    #[default]
    Hijack,
    Native,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "TunConfigWire")]
pub struct TunConfig {
    pub enabled: bool,
    pub interface_name: String,
    pub mtu: u16,
    pub address_v4: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_v6: Option<String>,
    pub stack: TunStack,
    pub strict_route: bool,
    pub dns_hijack: DnsHijackMode,
    #[serde(default)]
    pub exclude_routes: Vec<String>,
    #[serde(default)]
    pub exclude_processes: Vec<String>,
    #[serde(default)]
    pub exclude_domains: Vec<String>,
}

fn default_interface_name() -> String {
    "tun0".to_string()
}

fn default_mtu() -> u16 {
    1500
}

fn default_address_v4() -> String {
    "172.19.0.1/30".to_string()
}

fn default_strict_route() -> bool {
    true
}

impl Default for TunConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interface_name: default_interface_name(),
            mtu: default_mtu(),
            address_v4: default_address_v4(),
            address_v6: None,
            stack: TunStack::System,
            strict_route: true,
            dns_hijack: DnsHijackMode::Hijack,
            exclude_routes: Vec::new(),
            exclude_processes: Vec::new(),
            exclude_domains: Vec::new(),
        }
    }
}

impl TunConfig {
    /// All configured interface addresses: the IPv4 CIDR, plus the IPv6 CIDR when set.
    pub fn addresses(&self) -> Vec<String> {
        let mut addrs = vec![self.address_v4.clone()];
        if let Some(v6) = &self.address_v6 {
            addrs.push(v6.clone());
        }
        addrs
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_tun_interface_name(&self.interface_name)?;
        if !(576..=9000).contains(&self.mtu) {
            return Err(ValidationError::InvalidTunMtu(self.mtu));
        }
        validate_ip_cidr(&self.address_v4)?;
        if let Some(v6) = &self.address_v6 {
            validate_ip_cidr(v6)?;
        }
        for route in &self.exclude_routes {
            validate_ip_cidr(route)?;
        }
        for domain in &self.exclude_domains {
            validate_domain_pattern(domain)?;
        }
        for proc in &self.exclude_processes {
            if proc.is_empty() || proc.contains('/') || proc.contains('\\') {
                return Err(ValidationError::InvalidProcessName(proc.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct TunConfigWire {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_interface_name")]
    interface_name: String,
    #[serde(default = "default_mtu")]
    mtu: u16,
    #[serde(default = "default_address_v4")]
    address_v4: String,
    #[serde(default)]
    address_v6: Option<String>,
    #[serde(default)]
    stack: TunStack,
    #[serde(default = "default_strict_route")]
    strict_route: bool,
    #[serde(default)]
    dns_hijack: DnsHijackMode,
    #[serde(default)]
    exclude_routes: Vec<String>,
    #[serde(default)]
    exclude_processes: Vec<String>,
    #[serde(default)]
    exclude_domains: Vec<String>,
}

impl From<TunConfigWire> for TunConfig {
    fn from(wire: TunConfigWire) -> Self {
        Self {
            enabled: wire.enabled,
            interface_name: wire.interface_name,
            mtu: wire.mtu,
            address_v4: wire.address_v4,
            address_v6: wire.address_v6,
            stack: wire.stack,
            strict_route: wire.strict_route,
            dns_hijack: wire.dns_hijack,
            exclude_routes: wire.exclude_routes,
            exclude_processes: wire.exclude_processes,
            exclude_domains: wire.exclude_domains,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_disabled_with_documented_values() {
        let tun = TunConfig::default();
        assert!(!tun.enabled);
        assert_eq!(tun.interface_name, "tun0");
        assert_eq!(tun.mtu, 1500);
        assert_eq!(tun.address_v4, "172.19.0.1/30");
        assert!(tun.address_v6.is_none());
        assert_eq!(tun.stack, TunStack::System);
        assert!(tun.strict_route);
        assert_eq!(tun.dns_hijack, DnsHijackMode::Hijack);
        assert!(tun.exclude_routes.is_empty());
        assert!(tun.exclude_processes.is_empty());
        assert!(tun.exclude_domains.is_empty());
    }

    #[test]
    fn test_addresses_includes_v6_when_set() {
        let tun = TunConfig::default();
        assert_eq!(tun.addresses(), vec!["172.19.0.1/30".to_string()]);

        let tun = TunConfig {
            address_v6: Some("fd00::1/126".to_string()),
            ..TunConfig::default()
        };
        assert_eq!(
            tun.addresses(),
            vec!["172.19.0.1/30".to_string(), "fd00::1/126".to_string()]
        );
    }

    #[test]
    fn test_partial_section_fills_defaults() {
        let tun: TunConfig = toml::from_str("enabled = true\n").unwrap();
        assert!(tun.enabled);
        assert_eq!(tun.interface_name, "tun0");
        assert_eq!(tun.mtu, 1500);
        assert_eq!(tun.address_v4, "172.19.0.1/30");
        assert!(tun.strict_route);
    }

    #[test]
    fn test_unknown_fields_ignored() {
        let tun: TunConfig =
            toml::from_str("enabled = true\nfuture_field = \"whatever\"\n").unwrap();
        assert!(tun.enabled);
    }

    #[test]
    fn test_round_trip() {
        let tun = TunConfig {
            enabled: true,
            interface_name: "wg-tun".to_string(),
            mtu: 9000,
            address_v4: "198.18.0.1/30".to_string(),
            address_v6: Some("fd00::1/126".to_string()),
            stack: TunStack::Gvisor,
            strict_route: false,
            dns_hijack: DnsHijackMode::Native,
            exclude_routes: vec!["192.168.0.0/16".to_string()],
            exclude_processes: vec!["cloudflared".to_string()],
            exclude_domains: vec!["example.com".to_string()],
        };
        let toml_str = toml::to_string(&tun).unwrap();
        let parsed: TunConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(tun, parsed);
    }

    #[test]
    fn test_stack_serde_literals() {
        assert_eq!(
            serde_json::to_string(&TunStack::System).unwrap(),
            "\"system\""
        );
        assert_eq!(
            serde_json::to_string(&TunStack::Gvisor).unwrap(),
            "\"gvisor\""
        );
        assert_eq!(
            serde_json::to_string(&TunStack::Mixed).unwrap(),
            "\"mixed\""
        );
    }

    #[test]
    fn test_dns_hijack_serde_literals() {
        assert_eq!(
            serde_json::to_string(&DnsHijackMode::Hijack).unwrap(),
            "\"hijack\""
        );
        assert_eq!(
            serde_json::to_string(&DnsHijackMode::Native).unwrap(),
            "\"native\""
        );
        assert_eq!(
            serde_json::to_string(&DnsHijackMode::Disabled).unwrap(),
            "\"disabled\""
        );
    }

    #[test]
    fn test_validate_accepts_defaults() {
        assert!(TunConfig::default().validate().is_ok());
    }

    #[test]
    fn test_validate_interface_name() {
        let cases = [
            ("tun0", true),
            ("wg-tun_1", true),
            ("a", true),
            ("0123456789abcde", true),   // 15 chars
            ("0123456789abcdef", false), // 16 chars
            ("", false),
            ("Tun0", false),  // uppercase
            ("tun 0", false), // space
            ("tun.0", false), // dot
        ];
        for (name, valid) in cases {
            let tun = TunConfig {
                interface_name: name.to_string(),
                ..TunConfig::default()
            };
            assert_eq!(tun.validate().is_ok(), valid, "name={name}");
        }
    }

    #[test]
    fn test_validate_mtu_bounds() {
        for (mtu, valid) in [
            (575u16, false),
            (576, true),
            (1500, true),
            (9000, true),
            (9001, false),
        ] {
            let tun = TunConfig {
                mtu,
                ..TunConfig::default()
            };
            assert_eq!(tun.validate().is_ok(), valid, "mtu={mtu}");
        }
    }

    #[test]
    fn test_validate_rejects_bad_cidrs() {
        let bad_v4 = TunConfig {
            address_v4: "not-a-cidr".to_string(),
            ..TunConfig::default()
        };
        assert!(matches!(
            bad_v4.validate(),
            Err(ValidationError::InvalidIpCidr(_))
        ));

        let bad_exclude = TunConfig {
            exclude_routes: vec!["192.168.0.0/16".to_string(), "bogus".to_string()],
            ..TunConfig::default()
        };
        assert!(matches!(
            bad_exclude.validate(),
            Err(ValidationError::InvalidIpCidr(_))
        ));
    }

    #[test]
    fn test_legacy_section_without_new_fields_loads_empty() {
        let tun: TunConfig = toml::from_str("enabled = true\ninterface_name = \"utun\"\n").unwrap();
        assert!(tun.enabled);
        assert_eq!(tun.interface_name, "utun");
        assert!(tun.exclude_routes.is_empty());
        assert!(tun.exclude_processes.is_empty());
        assert!(tun.exclude_domains.is_empty());
    }

    #[test]
    fn test_validate_rejects_bad_domains() {
        let bad = TunConfig {
            exclude_domains: vec!["example.com".to_string(), "no-dot".to_string()],
            ..TunConfig::default()
        };
        assert!(matches!(
            bad.validate(),
            Err(ValidationError::InvalidDomainPattern(_))
        ));
    }

    #[test]
    fn test_validate_rejects_bad_process_names() {
        let bad = TunConfig {
            exclude_processes: vec!["cloudflared".to_string(), "bad/name".to_string()],
            ..TunConfig::default()
        };
        assert!(matches!(
            bad.validate(),
            Err(ValidationError::InvalidProcessName(_))
        ));

        let empty = TunConfig {
            exclude_processes: vec!["".to_string()],
            ..TunConfig::default()
        };
        assert!(matches!(
            empty.validate(),
            Err(ValidationError::InvalidProcessName(_))
        ));
    }
}
