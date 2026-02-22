use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DnsProtocol {
    Plain,
    DoH,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DnsServer {
    pub protocol: DnsProtocol,
    pub address: String,
}

impl DnsServer {
    pub fn server_address(&self) -> String {
        match self.protocol {
            DnsProtocol::Plain => self.address.clone(),
            DnsProtocol::DoH => {
                if self.address.starts_with("https://") {
                    self.address.clone()
                } else {
                    format!("https://{}/dns-query", self.address)
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DnsConfig {
    pub enabled: bool,
    pub remote: DnsServer,
    pub domestic: DnsServer,
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            remote: DnsServer {
                protocol: DnsProtocol::DoH,
                address: "1.1.1.1".into(),
            },
            domestic: DnsServer {
                protocol: DnsProtocol::Plain,
                address: "223.5.5.5".into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_address_plain() {
        let s = DnsServer {
            protocol: DnsProtocol::Plain,
            address: "223.5.5.5".into(),
        };
        assert_eq!(s.server_address(), "223.5.5.5");
    }

    #[test]
    fn test_server_address_doh() {
        let s = DnsServer {
            protocol: DnsProtocol::DoH,
            address: "1.1.1.1".into(),
        };
        assert_eq!(s.server_address(), "https://1.1.1.1/dns-query");
    }

    #[test]
    fn test_server_address_doh_full_url() {
        let s = DnsServer {
            protocol: DnsProtocol::DoH,
            address: "https://dns.adguard.com/dns-query".into(),
        };
        assert_eq!(s.server_address(), "https://dns.adguard.com/dns-query");
    }

    #[test]
    fn test_default_dns_config() {
        let cfg = DnsConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.remote.protocol, DnsProtocol::DoH);
        assert_eq!(cfg.remote.address, "1.1.1.1");
        assert_eq!(cfg.domestic.protocol, DnsProtocol::Plain);
        assert_eq!(cfg.domestic.address, "223.5.5.5");
    }

    #[test]
    fn test_dns_config_roundtrip() {
        let cfg = DnsConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: DnsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }
}
