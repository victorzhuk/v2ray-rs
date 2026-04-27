use uuid::Uuid;

use crate::fs::atomic_write;
use crate::models::ManualNode;

use super::{AppPaths, PersistenceError, read_file};

pub fn save_manual_nodes(paths: &AppPaths, nodes: &[ManualNode]) -> Result<(), PersistenceError> {
    paths.ensure_dirs()?;
    let json = serde_json::to_string_pretty(nodes)?;
    atomic_write(&paths.custom_nodes_path(), json.as_bytes()).map_err(PersistenceError::Io)
}

pub fn load_manual_nodes(paths: &AppPaths) -> Result<Vec<ManualNode>, PersistenceError> {
    let path = paths.custom_nodes_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = read_file(&path)?;
    let nodes: Vec<ManualNode> = serde_json::from_str(&contents)?;
    Ok(nodes)
}

pub fn load_manual_nodes_or_default(paths: &AppPaths) -> Vec<ManualNode> {
    match load_manual_nodes(paths) {
        Ok(nodes) => nodes,
        Err(PersistenceError::Json(_)) => {
            log::warn!("corrupt custom_nodes.json file; using empty manual nodes list");
            Vec::new()
        }
        Err(e) => {
            log::warn!("failed to load manual nodes: {e}; using empty list");
            Vec::new()
        }
    }
}

pub fn add_manual_node(paths: &AppPaths, node: ManualNode) -> Result<(), PersistenceError> {
    let mut nodes = load_manual_nodes(paths)?;
    nodes.push(node);
    save_manual_nodes(paths, &nodes)
}

pub fn get_manual_node(
    paths: &AppPaths,
    id: &Uuid,
) -> Result<Option<ManualNode>, PersistenceError> {
    let nodes = load_manual_nodes(paths)?;
    Ok(nodes.into_iter().find(|n| &n.id == id))
}

pub fn update_manual_node(paths: &AppPaths, node: ManualNode) -> Result<bool, PersistenceError> {
    let mut nodes = load_manual_nodes(paths)?;
    match nodes.iter_mut().find(|n| n.id == node.id) {
        Some(existing) => {
            *existing = node;
            save_manual_nodes(paths, &nodes)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

pub fn remove_manual_node(paths: &AppPaths, id: &Uuid) -> Result<bool, PersistenceError> {
    let mut nodes = load_manual_nodes(paths)?;
    let initial_len = nodes.len();
    nodes.retain(|n| &n.id != id);
    if nodes.len() < initial_len {
        save_manual_nodes(paths, &nodes)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::models::TransportSettings;
    use crate::models::{ManualNode, ProxyNode, ShadowsocksConfig, VlessConfig};
    use uuid::Uuid;

    #[test]
    fn test_manual_nodes_save_load_roundtrip() {
        let (_tmp, paths) = super::super::test_paths();

        let node = ProxyNode::Vless(VlessConfig {
            address: "test.com".into(),
            port: 443,
            uuid: "test-uuid".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: Some("Manual Test".into()),
        });
        let manual = ManualNode::new(node);

        save_manual_nodes(&paths, std::slice::from_ref(&manual)).unwrap();
        let loaded = load_manual_nodes(&paths).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, manual.id);
        assert_eq!(loaded[0].enabled, manual.enabled);
    }

    #[test]
    fn test_load_manual_nodes_missing_file() {
        let (_tmp, paths) = super::super::test_paths();
        paths.ensure_dirs().unwrap();
        let loaded = load_manual_nodes(&paths).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_add_manual_node() {
        let (_tmp, paths) = super::super::test_paths();

        let node1 = ProxyNode::Vless(VlessConfig {
            address: "a.com".into(),
            port: 443,
            uuid: "uuid1".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: None,
        });
        let node2 = ProxyNode::Vless(VlessConfig {
            address: "b.com".into(),
            port: 443,
            uuid: "uuid2".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: None,
        });

        add_manual_node(&paths, ManualNode::new(node1)).unwrap();
        add_manual_node(&paths, ManualNode::new(node2)).unwrap();

        let loaded = load_manual_nodes(&paths).unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn test_get_manual_node() {
        let (_tmp, paths) = super::super::test_paths();

        let node = ProxyNode::Vless(VlessConfig {
            address: "test.com".into(),
            port: 443,
            uuid: "test-uuid".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: None,
        });
        let manual = ManualNode::new(node);

        add_manual_node(&paths, manual.clone()).unwrap();

        let found = get_manual_node(&paths, &manual.id).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, manual.id);

        let not_found = get_manual_node(&paths, &Uuid::new_v4()).unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_update_manual_node() {
        let (_tmp, paths) = super::super::test_paths();

        let node = ProxyNode::Vless(VlessConfig {
            address: "test.com".into(),
            port: 443,
            uuid: "test-uuid".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: None,
        });
        let mut manual = ManualNode::new(node);

        add_manual_node(&paths, manual.clone()).unwrap();

        if let ProxyNode::Vless(ref mut cfg) = manual.node {
            cfg.address = "updated.com".into();
        }
        manual.enabled = false;

        let updated = update_manual_node(&paths, manual.clone()).unwrap();
        assert!(updated);

        let loaded = get_manual_node(&paths, &manual.id).unwrap().unwrap();
        assert!(!loaded.enabled);
        assert_eq!(loaded.node.address(), "updated.com");
    }

    #[test]
    fn test_remove_manual_node() {
        let (_tmp, paths) = super::super::test_paths();

        let node1 = ProxyNode::Vless(VlessConfig {
            address: "a.com".into(),
            port: 443,
            uuid: "uuid1".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: None,
        });
        let node2 = ProxyNode::Vless(VlessConfig {
            address: "b.com".into(),
            port: 443,
            uuid: "uuid2".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: None,
        });

        add_manual_node(&paths, ManualNode::new(node1)).unwrap();
        add_manual_node(&paths, ManualNode::new(node2)).unwrap();

        let loaded = load_manual_nodes(&paths).unwrap();
        let id = loaded[0].id;

        let removed = remove_manual_node(&paths, &id).unwrap();
        assert!(removed);

        let after = load_manual_nodes(&paths).unwrap();
        assert_eq!(after.len(), 1);
    }

    #[test]
    fn test_remove_nonexistent_manual_node() {
        let (_tmp, paths) = super::super::test_paths();
        let removed = remove_manual_node(&paths, &Uuid::new_v4()).unwrap();
        assert!(!removed);
    }

    #[test]
    fn test_load_manual_nodes_or_default_with_corrupt_file() {
        let (_tmp, paths) = super::super::test_paths();
        paths.ensure_dirs().unwrap();
        fs::write(paths.custom_nodes_path(), "invalid {{{{json").unwrap();

        let loaded = load_manual_nodes_or_default(&paths);
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_load_manual_nodes_or_default_with_missing_file() {
        let (_tmp, paths) = super::super::test_paths();
        paths.ensure_dirs().unwrap();

        let loaded = load_manual_nodes_or_default(&paths);
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_manual_nodes_roundtrip_with_real_data() {
        let (_tmp, paths) = super::super::test_paths();

        let ss_node = ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: "ss.example.com".into(),
            port: 8388,
            method: "aes-256-gcm".into(),
            password: "test-pass".into(),
            remark: Some("SS Test".into()),
        });

        let manual1 = ManualNode::new(ss_node);

        save_manual_nodes(&paths, std::slice::from_ref(&manual1)).unwrap();

        let loaded = load_manual_nodes(&paths).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, manual1.id);
        assert_eq!(loaded[0].node, manual1.node);
        assert_eq!(loaded[0].enabled, manual1.enabled);
    }
}
