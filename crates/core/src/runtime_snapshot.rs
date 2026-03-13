use std::path::PathBuf;

use crate::models::{AppSettings, BackendType, DnsConfig, RoutingRuleSet};

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeConfigSnapshot {
    pub backend_type: BackendType,
    pub binary_path: Option<PathBuf>,
    pub socks_port: u16,
    pub http_port: u16,
    pub dns: DnsConfig,
    pub routing: RoutingRuleSet,
    pub timestamp: i64,
}

impl RuntimeConfigSnapshot {
    pub fn new(
        backend_type: BackendType,
        binary_path: Option<PathBuf>,
        socks_port: u16,
        http_port: u16,
        dns: DnsConfig,
        routing: RoutingRuleSet,
        timestamp: i64,
    ) -> Self {
        Self {
            backend_type,
            binary_path,
            socks_port,
            http_port,
            dns,
            routing,
            timestamp,
        }
    }

    pub fn diverges_from(&self, settings: &AppSettings, routing: &RoutingRuleSet) -> bool {
        self.backend_type != settings.backend.backend_type
            || self.binary_path != settings.backend.binary_path
            || self.socks_port != settings.socks_port
            || self.http_port != settings.http_port
            || self.dns != settings.dns
            || self.routing != *routing
    }

    pub fn restore_settings(&self, settings: &mut AppSettings) {
        settings.backend.backend_type = self.backend_type;
        settings.backend.binary_path = self.binary_path.clone();
        settings.socks_port = self.socks_port;
        settings.http_port = self.http_port;
        settings.dns = self.dns.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Language;

    #[test]
    fn test_runtime_config_snapshot_creation() {
        let snapshot = RuntimeConfigSnapshot::new(
            BackendType::Xray,
            Some(PathBuf::from("/usr/bin/xray")),
            1080,
            1081,
            DnsConfig::default(),
            RoutingRuleSet::new(),
            1234567890,
        );

        assert_eq!(snapshot.backend_type, BackendType::Xray);
        assert_eq!(snapshot.binary_path, Some(PathBuf::from("/usr/bin/xray")));
        assert_eq!(snapshot.socks_port, 1080);
        assert_eq!(snapshot.http_port, 1081);
        assert_eq!(snapshot.timestamp, 1234567890);
    }

    #[test]
    fn test_runtime_config_snapshot_equality() {
        let snapshot1 = RuntimeConfigSnapshot::new(
            BackendType::Xray,
            Some(PathBuf::from("/usr/bin/xray")),
            1080,
            1081,
            DnsConfig::default(),
            RoutingRuleSet::new(),
            1234567890,
        );

        let snapshot2 = RuntimeConfigSnapshot::new(
            BackendType::Xray,
            Some(PathBuf::from("/usr/bin/xray")),
            1080,
            1081,
            DnsConfig::default(),
            RoutingRuleSet::new(),
            1234567890,
        );

        assert_eq!(snapshot1, snapshot2);
    }

    #[test]
    fn test_runtime_config_snapshot_inequality() {
        let snapshot1 = RuntimeConfigSnapshot::new(
            BackendType::Xray,
            Some(PathBuf::from("/usr/bin/xray")),
            1080,
            1081,
            DnsConfig::default(),
            RoutingRuleSet::new(),
            1234567890,
        );

        let snapshot2 = RuntimeConfigSnapshot::new(
            BackendType::V2ray,
            Some(PathBuf::from("/usr/bin/v2ray")),
            1080,
            1081,
            DnsConfig::default(),
            RoutingRuleSet::new(),
            1234567890,
        );

        assert_ne!(snapshot1, snapshot2);
    }

    #[test]
    fn test_runtime_config_snapshot_detects_runtime_divergence() {
        let snapshot = RuntimeConfigSnapshot::new(
            BackendType::Xray,
            Some(PathBuf::from("/usr/bin/xray")),
            1080,
            1081,
            DnsConfig::default(),
            RoutingRuleSet::new(),
            1234567890,
        );

        let mut settings = AppSettings::default();
        settings.backend.backend_type = BackendType::Xray;
        settings.backend.binary_path = Some(PathBuf::from("/usr/bin/xray"));

        assert!(!snapshot.diverges_from(&settings, &RoutingRuleSet::new()));

        settings.http_port = 2081;
        assert!(snapshot.diverges_from(&settings, &RoutingRuleSet::new()));
    }

    #[test]
    fn test_runtime_config_snapshot_restore_only_updates_runtime_fields() {
        let snapshot = RuntimeConfigSnapshot::new(
            BackendType::SingBox,
            Some(PathBuf::from("/usr/bin/sing-box")),
            2080,
            2081,
            DnsConfig::default(),
            RoutingRuleSet::new(),
            1234567890,
        );

        let mut settings = AppSettings::default();
        settings.language = Language::Russian;
        settings.minimize_to_tray = false;

        snapshot.restore_settings(&mut settings);

        assert_eq!(settings.backend.backend_type, BackendType::SingBox);
        assert_eq!(
            settings.backend.binary_path,
            Some(PathBuf::from("/usr/bin/sing-box"))
        );
        assert_eq!(settings.socks_port, 2080);
        assert_eq!(settings.http_port, 2081);
        assert_eq!(settings.language, Language::Russian);
        assert!(!settings.minimize_to_tray);
    }
}
