use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

use crate::models::{AutoResolveStrategy, DnsConfig, LastSuccessMetadata};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    pub version: u32,
    pub backend: BackendConfig,
    pub socks_port: u16,
    pub http_port: u16,
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
    fn test_geo_aware_strategy_migrates_to_last_successful() {
        let settings: AppSettings =
            toml::from_str("version = 1\nsocks_port = 1080\nhttp_port = 1081\nauto_update_subscriptions = true\nsubscription_update_interval_secs = 86400\nauto_update_geodata = true\ngeodata_update_interval_secs = 604800\nlanguage = \"english\"\nminimize_to_tray = true\nnotifications_enabled = true\nonboarding_complete = false\nauto_resolve_strategy = \"geo-aware\"\n[backend]\nbackend_type = \"xray\"\n[dns]\nenabled = false\n").unwrap();

        assert_eq!(
            settings.auto_resolve_strategy,
            AutoResolveStrategy::LastSuccessful
        );
    }
}
