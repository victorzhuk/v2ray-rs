use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    V2ray,
    Xray,
    SingBox,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    English,
    Russian,
}

impl Default for Language {
    fn default() -> Self {
        Self::English
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    pub version: u32,
    pub backend: BackendConfig,
    pub socks_port: u16,
    pub http_port: u16,
    pub auto_update_subscriptions: bool,
    pub subscription_update_interval_secs: u64,
    pub auto_update_geodata: bool,
    pub geodata_update_interval_secs: u64,
    pub language: Language,
    pub minimize_to_tray: bool,
    pub notifications_enabled: bool,
    pub onboarding_complete: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            backend: BackendConfig::default(),
            socks_port: 1080,
            http_port: 1081,
            auto_update_subscriptions: true,
            subscription_update_interval_secs: 86400,
            auto_update_geodata: true,
            geodata_update_interval_secs: 604800,
            language: Language::English,
            minimize_to_tray: true,
            notifications_enabled: true,
            onboarding_complete: false,
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
}
