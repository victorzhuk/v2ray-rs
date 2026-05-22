use std::path::PathBuf;

use crate::models::{
    AppSettings, BackendType, DnsConfig, ManualNode, RoutingRuleSet, Subscription,
};

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeConfigSnapshot {
    pub backend_type: BackendType,
    pub binary_path: Option<PathBuf>,
    pub socks_port: u16,
    pub http_port: u16,
    pub listen_address: String,
    pub dns: DnsConfig,
    pub routing: RoutingRuleSet,
    pub manual_nodes: Vec<ManualNode>,
    pub subscriptions: Vec<Subscription>,
    pub timestamp: i64,
}

impl RuntimeConfigSnapshot {
    pub fn diverges_from(
        &self,
        settings: &AppSettings,
        routing: &RoutingRuleSet,
        manual_nodes: &[ManualNode],
        subscriptions: &[Subscription],
    ) -> bool {
        self.backend_type != settings.backend.backend_type
            || self.binary_path != settings.backend.binary_path
            || self.socks_port != settings.socks_port
            || self.http_port != settings.http_port
            || self.listen_address != settings.listen_address
            || self.dns != settings.dns
            || self.routing != *routing
            || self.manual_nodes != manual_nodes
            || !subscriptions_runtime_state_eq(&self.subscriptions, subscriptions)
    }

    pub fn restore_settings(&self, settings: &mut AppSettings) {
        settings.backend.backend_type = self.backend_type;
        settings.backend.binary_path = self.binary_path.clone();
        settings.socks_port = self.socks_port;
        settings.http_port = self.http_port;
        settings.listen_address = self.listen_address.clone();
        settings.dns = self.dns.clone();
    }

    pub fn restore_manual_nodes(&self) -> Vec<ManualNode> {
        self.manual_nodes.clone()
    }

    pub fn restore_subscriptions(&self) -> Vec<Subscription> {
        self.subscriptions.clone()
    }
}

pub fn subscriptions_runtime_state_eq(lhs: &[Subscription], rhs: &[Subscription]) -> bool {
    lhs.len() == rhs.len()
        && lhs
            .iter()
            .zip(rhs)
            .all(|(left, right)| left.runtime_state_eq(right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Language;

    fn make_snapshot(backend_type: BackendType, binary_path: &str) -> RuntimeConfigSnapshot {
        RuntimeConfigSnapshot {
            backend_type,
            binary_path: Some(PathBuf::from(binary_path)),
            socks_port: 1080,
            http_port: 1081,
            listen_address: "127.0.0.1".to_string(),
            dns: DnsConfig::default(),
            routing: RoutingRuleSet::new(),
            manual_nodes: Vec::new(),
            subscriptions: Vec::new(),
            timestamp: 1234567890,
        }
    }

    #[test]
    fn test_runtime_config_snapshot_creation() {
        let snapshot = make_snapshot(BackendType::Xray, "/usr/bin/xray");

        assert_eq!(snapshot.backend_type, BackendType::Xray);
        assert_eq!(snapshot.binary_path, Some(PathBuf::from("/usr/bin/xray")));
        assert_eq!(snapshot.socks_port, 1080);
        assert_eq!(snapshot.http_port, 1081);
        assert_eq!(snapshot.listen_address, "127.0.0.1");
        assert_eq!(snapshot.timestamp, 1234567890);
    }

    #[test]
    fn test_runtime_config_snapshot_equality() {
        let snapshot1 = make_snapshot(BackendType::Xray, "/usr/bin/xray");
        let snapshot2 = make_snapshot(BackendType::Xray, "/usr/bin/xray");

        assert_eq!(snapshot1, snapshot2);
    }

    #[test]
    fn test_runtime_config_snapshot_inequality() {
        let snapshot1 = make_snapshot(BackendType::Xray, "/usr/bin/xray");
        let snapshot2 = make_snapshot(BackendType::V2ray, "/usr/bin/v2ray");

        assert_ne!(snapshot1, snapshot2);
    }

    #[test]
    fn test_runtime_config_snapshot_detects_runtime_divergence() {
        let snapshot = make_snapshot(BackendType::Xray, "/usr/bin/xray");

        let mut settings = AppSettings {
            backend: crate::models::BackendConfig {
                backend_type: BackendType::Xray,
                binary_path: Some(PathBuf::from("/usr/bin/xray")),
                ..crate::models::BackendConfig::default()
            },
            ..AppSettings::default()
        };

        assert!(!snapshot.diverges_from(&settings, &RoutingRuleSet::new(), &[], &[]));

        settings.http_port = 2081;
        assert!(snapshot.diverges_from(&settings, &RoutingRuleSet::new(), &[], &[]));

        settings.listen_address = "0.0.0.0".to_string();
        assert!(snapshot.diverges_from(&settings, &RoutingRuleSet::new(), &[], &[]));
    }

    #[test]
    fn test_runtime_config_snapshot_restore_only_updates_runtime_fields() {
        let snapshot = RuntimeConfigSnapshot {
            backend_type: BackendType::SingBox,
            binary_path: Some(PathBuf::from("/usr/bin/sing-box")),
            socks_port: 2080,
            http_port: 2081,
            listen_address: "0.0.0.0".to_string(),
            dns: DnsConfig::default(),
            routing: RoutingRuleSet::new(),
            manual_nodes: Vec::new(),
            subscriptions: Vec::new(),
            timestamp: 1234567890,
        };

        let mut settings = AppSettings {
            language: Language::Russian,
            minimize_to_tray: false,
            ..AppSettings::default()
        };

        snapshot.restore_settings(&mut settings);

        assert_eq!(settings.backend.backend_type, BackendType::SingBox);
        assert_eq!(
            settings.backend.binary_path,
            Some(PathBuf::from("/usr/bin/sing-box"))
        );
        assert_eq!(settings.socks_port, 2080);
        assert_eq!(settings.http_port, 2081);
        assert_eq!(settings.listen_address, "0.0.0.0");
        assert_eq!(settings.language, Language::Russian);
        assert!(!settings.minimize_to_tray);
    }

    #[test]
    fn test_runtime_config_snapshot_detects_manual_node_divergence() {
        use crate::models::{ProxyNode, TransportSettings, VlessConfig};

        let snapshot = RuntimeConfigSnapshot {
            backend_type: BackendType::Xray,
            binary_path: Some(PathBuf::from("/usr/bin/xray")),
            socks_port: 1080,
            http_port: 1081,
            listen_address: "127.0.0.1".to_string(),
            dns: DnsConfig::default(),
            routing: RoutingRuleSet::new(),
            manual_nodes: vec![ManualNode::with_id(
                uuid::Uuid::nil(),
                ProxyNode::Vless(VlessConfig {
                    address: "example.com".into(),
                    port: 443,
                    uuid: "snapshot-node".into(),
                    encryption: None,
                    flow: None,
                    transport: TransportSettings::Tcp,
                    tls: None,
                    remark: Some("Snapshot".into()),
                }),
                true,
            )],
            subscriptions: Vec::new(),
            timestamp: 1234567890,
        };

        let settings = AppSettings {
            backend: crate::models::BackendConfig {
                backend_type: BackendType::Xray,
                binary_path: Some(PathBuf::from("/usr/bin/xray")),
                ..crate::models::BackendConfig::default()
            },
            ..AppSettings::default()
        };

        assert!(!snapshot.diverges_from(
            &settings,
            &RoutingRuleSet::new(),
            &snapshot.manual_nodes,
            &[]
        ));

        let changed_nodes = vec![ManualNode::with_id(
            uuid::Uuid::nil(),
            ProxyNode::Vless(VlessConfig {
                address: "changed.example.com".into(),
                port: 443,
                uuid: "snapshot-node".into(),
                encryption: None,
                flow: None,
                transport: TransportSettings::Tcp,
                tls: None,
                remark: Some("Snapshot".into()),
            }),
            true,
        )];

        assert!(snapshot.diverges_from(&settings, &RoutingRuleSet::new(), &changed_nodes, &[]));
    }

    #[test]
    fn test_runtime_config_snapshot_restores_manual_nodes() {
        use crate::models::{ProxyNode, TransportSettings, VlessConfig};

        let manual_nodes = vec![ManualNode::with_id(
            uuid::Uuid::nil(),
            ProxyNode::Vless(VlessConfig {
                address: "example.com".into(),
                port: 443,
                uuid: "snapshot-node".into(),
                encryption: None,
                flow: None,
                transport: TransportSettings::Tcp,
                tls: None,
                remark: Some("Snapshot".into()),
            }),
            true,
        )];

        let snapshot = RuntimeConfigSnapshot {
            backend_type: BackendType::Xray,
            binary_path: Some(PathBuf::from("/usr/bin/xray")),
            socks_port: 1080,
            http_port: 1081,
            listen_address: "127.0.0.1".to_string(),
            dns: DnsConfig::default(),
            routing: RoutingRuleSet::new(),
            manual_nodes: manual_nodes.clone(),
            subscriptions: Vec::new(),
            timestamp: 1234567890,
        };

        assert_eq!(snapshot.restore_manual_nodes(), manual_nodes);
    }
}
