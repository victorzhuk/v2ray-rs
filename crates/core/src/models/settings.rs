use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};

use crate::models::{
    AutoResolveStrategy, DnsConfig, LastSuccessMetadata, TunConfig, ValidationError,
    validate_test_url,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    V2ray,
    Xray,
    SingBox,
}

impl fmt::Display for BackendType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendType::V2ray => f.write_str("v2ray"),
            BackendType::Xray => f.write_str("xray"),
            BackendType::SingBox => f.write_str("sing-box"),
        }
    }
}

impl BackendType {
    pub fn to_index(self) -> u32 {
        match self {
            BackendType::V2ray => 0,
            BackendType::Xray => 1,
            BackendType::SingBox => 2,
        }
    }

    pub fn from_index(index: u32) -> Option<Self> {
        match index {
            0 => Some(BackendType::V2ray),
            1 => Some(BackendType::Xray),
            2 => Some(BackendType::SingBox),
            _ => None,
        }
    }

    /// Whether this backend can perform a Real Delay probe.
    ///
    /// sing-box uses its Clash API; xray and v2ray use their ObservatoryService
    /// over gRPC. All three backends have probe config generators registered.
    #[must_use]
    pub fn supports_real_delay(self) -> bool {
        true // All supported backends have probe generators
    }

    /// Returns the default Real Delay capability state for this backend type.
    ///
    /// For xray and v2ray, capability is checked on first run (PotentiallySupported).
    /// For sing-box, we assume support (Supported) as it uses the standard Clash API.
    #[must_use]
    pub const fn default_real_delay_capability(self) -> RealDelayCapability {
        match self {
            BackendType::V2ray | BackendType::Xray => RealDelayCapability::PotentiallySupported {
                requirement: "ObservatoryService",
            },
            BackendType::SingBox => RealDelayCapability::Supported,
        }
    }
}

/// Runtime capability state for Real Delay, tracked per-session.
///
/// This is runtime-only state, not persisted to disk. It tracks whether
/// the backend actually supports the required probe surface (ObservatoryService
/// for xray/v2ray, Clash API for sing-box).
#[derive(Debug, Clone, PartialEq)]
pub enum RealDelayCapability {
    /// Backend type supports Real Delay in principle; capability will be checked on first run.
    PotentiallySupported { requirement: &'static str },
    /// A previous run confirmed the backend has the required probe surface.
    Supported,
    /// A previous run found the backend lacks the required probe surface.
    Unsupported { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendConfig {
    pub backend_type: BackendType,
    pub binary_path: Option<PathBuf>,
    pub config_output_dir: Option<PathBuf>,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            backend_type: BackendType::Xray,
            binary_path: None,
            config_output_dir: None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    English,
    Russian,
}

/// User preferences for the on-demand Real Delay latency probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealDelaySettings {
    pub enabled: bool,
    pub test_url: String,
    pub timeout_ms: u32,
    pub use_for_lowest_latency: bool,
}

pub fn default_real_delay_test_url() -> String {
    "https://www.gstatic.com/generate_204".to_string()
}

impl Default for RealDelaySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            test_url: default_real_delay_test_url(),
            timeout_ms: 5000,
            use_for_lowest_latency: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    pub version: u32,
    pub backend: BackendConfig,
    pub socks_port: u16,
    pub http_port: u16,
    #[serde(default = "default_listen_address")]
    pub listen_address: String,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u32,
    #[serde(default, deserialize_with = "deserialize_auto_resolve_strategy")]
    pub auto_resolve_strategy: AutoResolveStrategy,
    #[serde(default)]
    pub last_success: Option<LastSuccessMetadata>,
    pub auto_update_subscriptions: bool,
    pub subscription_update_interval_secs: u64,
    pub auto_update_geodata: bool,
    pub geodata_update_interval_secs: u64,
    pub language: Language,
    pub minimize_to_tray: bool,
    pub notifications_enabled: bool,
    pub onboarding_complete: bool,
    #[serde(default)]
    pub dns: DnsConfig,
    #[serde(default)]
    pub tun: TunConfig,
    #[serde(default)]
    pub real_delay: RealDelaySettings,
}

pub fn default_listen_address() -> String {
    "127.0.0.1".to_string()
}

pub fn default_idle_timeout_secs() -> u32 {
    600
}

impl AppSettings {
    /// Validates a listen-address string: must be a parseable IPv4 or IPv6 literal.
    /// Hostnames and empty strings are rejected.
    pub fn validate_listen_address(addr: &str) -> Result<(), ValidationError> {
        if addr.is_empty() {
            return Err(ValidationError::InvalidListenAddress(addr.to_string()));
        }
        IpAddr::from_str(addr)
            .map(|_| ())
            .map_err(|_| ValidationError::InvalidListenAddress(addr.to_string()))
    }

    /// Validates a Real Delay test URL: must be a syntactically valid
    /// `http://` or `https://` URL. Other schemes are rejected.
    pub fn validate_real_delay_url(url: &str) -> Result<(), ValidationError> {
        validate_test_url(url)
    }
}

fn deserialize_auto_resolve_strategy<'de, D>(
    deserializer: D,
) -> Result<AutoResolveStrategy, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    match s.as_str() {
        "list-order" => Ok(AutoResolveStrategy::ListOrder),
        "lowest-latency" => Ok(AutoResolveStrategy::LowestLatency),
        "random" => Ok(AutoResolveStrategy::Random),
        "last-successful" | "geo-aware" => Ok(AutoResolveStrategy::LastSuccessful),
        other => Err(serde::de::Error::unknown_variant(
            other,
            &["list-order", "lowest-latency", "random", "last-successful"],
        )),
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            backend: BackendConfig::default(),
            socks_port: 1080,
            http_port: 1081,
            listen_address: default_listen_address(),
            idle_timeout_secs: default_idle_timeout_secs(),
            auto_resolve_strategy: AutoResolveStrategy::default(),
            last_success: None,
            auto_update_subscriptions: true,
            subscription_update_interval_secs: 86400,
            auto_update_geodata: true,
            geodata_update_interval_secs: 604800,
            language: Language::English,
            minimize_to_tray: true,
            notifications_enabled: true,
            onboarding_complete: false,
            dns: DnsConfig::default(),
            tun: TunConfig::default(),
            real_delay: RealDelaySettings::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = AppSettings::default();
        assert_eq!(settings.socks_port, 1080);
        assert_eq!(settings.http_port, 1081);
        assert_eq!(
            settings.auto_resolve_strategy,
            AutoResolveStrategy::ListOrder
        );
        assert_eq!(settings.language, Language::English);
        assert_eq!(settings.version, 1);
        assert!(settings.auto_update_subscriptions);
        assert!(settings.minimize_to_tray);
        assert!(!settings.onboarding_complete);
    }

    #[test]
    fn test_default_backend() {
        let backend = BackendConfig::default();
        assert_eq!(backend.backend_type, BackendType::Xray);
        assert!(backend.binary_path.is_none());
        assert!(backend.config_output_dir.is_none());
    }

    #[test]
    fn test_settings_toml_roundtrip() {
        let settings = AppSettings::default();
        let toml_str = toml::to_string(&settings).unwrap();
        let deserialized: AppSettings = toml::from_str(&toml_str).unwrap();
        assert_eq!(settings, deserialized);
    }

    #[test]
    fn test_settings_json_roundtrip() {
        let settings = AppSettings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(settings, deserialized);
    }

    #[test]
    fn test_default_listen_address() {
        let settings = AppSettings::default();
        assert_eq!(settings.listen_address, "127.0.0.1");
    }

    #[test]
    fn test_legacy_settings_toml_missing_listen_address_defaults_to_loopback() {
        let toml_str = "version = 1\nsocks_port = 1080\nhttp_port = 1081\nauto_update_subscriptions = true\nsubscription_update_interval_secs = 86400\nauto_update_geodata = true\ngeodata_update_interval_secs = 604800\nlanguage = \"english\"\nminimize_to_tray = true\nnotifications_enabled = true\nonboarding_complete = false\n[backend]\nbackend_type = \"xray\"\n[dns]\nenabled = false\n";
        let settings: AppSettings = toml::from_str(toml_str).unwrap();
        assert_eq!(settings.listen_address, "127.0.0.1");
    }

    #[test]
    fn test_listen_address_round_trip() {
        let settings = AppSettings {
            listen_address: "0.0.0.0".to_string(),
            ..AppSettings::default()
        };
        let toml_str = toml::to_string(&settings).unwrap();
        let deserialized: AppSettings = toml::from_str(&toml_str).unwrap();
        assert_eq!(deserialized.listen_address, "0.0.0.0");
        assert_eq!(settings, deserialized);
    }

    #[test]
    fn test_validate_listen_address() {
        let valid = ["127.0.0.1", "0.0.0.0", "::", "::1", "192.168.1.10"];
        for addr in valid {
            assert!(
                AppSettings::validate_listen_address(addr).is_ok(),
                "expected {addr} to be valid"
            );
        }

        let invalid = ["", "localhost", "not-an-ip"];
        for addr in invalid {
            let result = AppSettings::validate_listen_address(addr);
            assert!(result.is_err(), "expected {addr} to be invalid");
            assert!(matches!(
                result,
                Err(ValidationError::InvalidListenAddress(_))
            ));
        }
    }

    #[test]
    fn test_default_real_delay_settings() {
        let settings = AppSettings::default();
        assert!(settings.real_delay.enabled);
        assert_eq!(
            settings.real_delay.test_url,
            "https://www.gstatic.com/generate_204"
        );
        assert_eq!(settings.real_delay.timeout_ms, 5000);
        assert!(!settings.real_delay.use_for_lowest_latency);
    }

    #[test]
    fn test_legacy_settings_toml_missing_real_delay_defaults() {
        let toml_str = "version = 1\nsocks_port = 1080\nhttp_port = 1081\nauto_update_subscriptions = true\nsubscription_update_interval_secs = 86400\nauto_update_geodata = true\ngeodata_update_interval_secs = 604800\nlanguage = \"english\"\nminimize_to_tray = true\nnotifications_enabled = true\nonboarding_complete = false\n[backend]\nbackend_type = \"xray\"\n[dns]\nenabled = false\n";
        let settings: AppSettings = toml::from_str(toml_str).unwrap();
        assert!(settings.real_delay.enabled);
        assert_eq!(
            settings.real_delay.test_url,
            "https://www.gstatic.com/generate_204"
        );
        assert_eq!(settings.real_delay.timeout_ms, 5000);
        assert!(!settings.real_delay.use_for_lowest_latency);
    }

    #[test]
    fn test_real_delay_settings_round_trip() {
        let settings = AppSettings {
            real_delay: RealDelaySettings {
                enabled: true,
                test_url: "https://cp.cloudflare.com/generate_204".to_string(),
                timeout_ms: 3000,
                use_for_lowest_latency: true,
            },
            ..AppSettings::default()
        };
        let toml_str = toml::to_string(&settings).unwrap();
        let deserialized: AppSettings = toml::from_str(&toml_str).unwrap();
        assert_eq!(deserialized.real_delay, settings.real_delay);
        assert_eq!(settings, deserialized);
    }

    #[test]
    fn test_validate_real_delay_url() {
        let valid = [
            "https://www.gstatic.com/generate_204",
            "http://cp.cloudflare.com/generate_204",
            "https://www.apple.com/library/test/success.html",
        ];
        for url in valid {
            assert!(
                AppSettings::validate_real_delay_url(url).is_ok(),
                "expected {url} to be valid"
            );
        }

        let invalid = ["not-a-url", "ftp://example.com/", "", "example.com"];
        for url in invalid {
            let result = AppSettings::validate_real_delay_url(url);
            assert!(result.is_err(), "expected {url} to be invalid");
            assert!(matches!(result, Err(ValidationError::InvalidTestUrl(_))));
        }
    }

    #[test]
    fn test_legacy_settings_toml_missing_tun_defaults_to_disabled() {
        let toml_str = "version = 1\nsocks_port = 1080\nhttp_port = 1081\nauto_update_subscriptions = true\nsubscription_update_interval_secs = 86400\nauto_update_geodata = true\ngeodata_update_interval_secs = 604800\nlanguage = \"english\"\nminimize_to_tray = true\nnotifications_enabled = true\nonboarding_complete = false\n[backend]\nbackend_type = \"xray\"\n[dns]\nenabled = false\n";
        let settings: AppSettings = toml::from_str(toml_str).unwrap();
        assert!(!settings.tun.enabled);
        assert_eq!(settings.tun.interface_name, "tun0");
        assert_eq!(settings.tun.mtu, 1500);
        assert_eq!(settings.tun.address_v4, "172.19.0.1/30");
        assert_eq!(settings.tun.stack, crate::models::TunStack::System);
    }

    #[test]
    fn test_tun_settings_round_trip() {
        let settings = AppSettings {
            tun: crate::models::TunConfig {
                enabled: true,
                interface_name: "vpn-tun".to_string(),
                mtu: 1400,
                address_v4: "198.18.0.1/30".to_string(),
                address_v6: None,
                stack: crate::models::TunStack::Gvisor,
                strict_route: false,
                dns_hijack: crate::models::DnsHijackMode::Native,
                exclude_routes: vec!["10.0.0.0/8".to_string()],
                exclude_processes: vec![],
                exclude_domains: vec![],
            },
            ..AppSettings::default()
        };
        let toml_str = toml::to_string(&settings).unwrap();
        let deserialized: AppSettings = toml::from_str(&toml_str).unwrap();
        assert_eq!(deserialized.tun, settings.tun);
        assert_eq!(settings, deserialized);
    }

    #[test]
    fn test_geo_aware_strategy_migrates_to_last_successful() {
        let settings: AppSettings =
            toml::from_str("version = 1\nsocks_port = 1080\nhttp_port = 1081\nauto_update_subscriptions = true\nsubscription_update_interval_secs = 86400\nauto_update_geodata = true\ngeodata_update_interval_secs = 604800\nlanguage = \"english\"\nminimize_to_tray = true\nnotifications_enabled = true\nonboarding_complete = false\nauto_resolve_strategy = \"geo-aware\"\n[backend]\nbackend_type = \"xray\"\n[dns]\nenabled = false\n").unwrap();

        assert_eq!(
            settings.auto_resolve_strategy,
            AutoResolveStrategy::LastSuccessful
        );
    }
}
