use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ProxyNode;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualNode {
    pub id: Uuid,
    pub node: ProxyNode,
    pub enabled: bool,
}

impl ManualNode {
    pub fn new(node: ProxyNode) -> Self {
        Self {
            id: Uuid::new_v4(),
            node,
            enabled: true,
        }
    }

    pub fn with_id(id: Uuid, node: ProxyNode, enabled: bool) -> Self {
        Self { id, node, enabled }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TransportSettings;
    use crate::models::VlessConfig;

    #[test]
    fn test_manual_node_new_generates_id() {
        let node = ProxyNode::Vless(VlessConfig {
            address: "example.com".into(),
            port: 443,
            uuid: "test-uuid".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: Some("Test".into()),
        });
        let manual = ManualNode::new(node.clone());
        assert_eq!(manual.node, node);
        assert!(manual.enabled);
    }

    #[test]
    fn test_manual_node_with_id() {
        let id = Uuid::new_v4();
        let node = ProxyNode::Vless(VlessConfig {
            address: "example.com".into(),
            port: 443,
            uuid: "test-uuid".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: Some("Test".into()),
        });
        let manual = ManualNode::with_id(id, node.clone(), false);
        assert_eq!(manual.id, id);
        assert_eq!(manual.node, node);
        assert!(!manual.enabled);
    }

    #[test]
    fn test_manual_node_serialization_roundtrip() {
        let node = ProxyNode::Vless(VlessConfig {
            address: "example.com".into(),
            port: 443,
            uuid: "test-uuid".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: Some("Test".into()),
        });
        let manual = ManualNode::new(node.clone());

        let json = serde_json::to_string(&manual).unwrap();
        let deserialized: ManualNode = serde_json::from_str(&json).unwrap();

        assert_eq!(manual.id, deserialized.id);
        assert_eq!(manual.node, deserialized.node);
        assert_eq!(manual.enabled, deserialized.enabled);
    }
}
