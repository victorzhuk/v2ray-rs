use serde_json::Value as JsonValue;

use crate::fs::atomic_write;
use crate::resolve::LatencySnapshot;

use super::settings::{SubscriptionNodeMap, load_subscription_node_map};
use super::{AppPaths, PersistenceError, RefMigration, json_uuid, read_file};

pub fn save_latency_snapshot(
    paths: &AppPaths,
    snapshot: &LatencySnapshot,
) -> Result<(), PersistenceError> {
    paths.ensure_dirs()?;
    let json = serde_json::to_string_pretty(snapshot)?;
    atomic_write(&paths.latency_snapshot_path(), json.as_bytes()).map_err(PersistenceError::Io)
}

pub fn load_latency_snapshot(paths: &AppPaths) -> Result<LatencySnapshot, PersistenceError> {
    let path = paths.latency_snapshot_path();
    if !path.exists() {
        return Ok(LatencySnapshot::default());
    }
    let contents = read_file(&path)?;
    let mut raw: JsonValue = serde_json::from_str(&contents)?;
    let node_ids = load_subscription_node_map(paths);
    let migrated = migrate_latency_snapshot_legacy_refs(&mut raw, &node_ids);
    let snapshot: LatencySnapshot = serde_json::from_value(raw)?;
    if migrated {
        save_latency_snapshot(paths, &snapshot)?;
    }
    Ok(snapshot)
}

fn migrate_latency_snapshot_legacy_refs(
    raw: &mut JsonValue,
    node_ids: &SubscriptionNodeMap,
) -> bool {
    let Some(root) = raw.as_object_mut() else {
        return false;
    };
    let Some(samples) = root.get_mut("samples").and_then(JsonValue::as_array_mut) else {
        return false;
    };

    let initial_len = samples.len();
    let mut changed = false;

    samples.retain_mut(|entry| {
        // Pre-migration: wrap old flat format into node_ref
        if let Some(obj) = entry.as_object_mut() {
            if !obj.contains_key("node_ref") {
                if obj.contains_key("subscription_id") && obj.contains_key("node_index") {
                    // Old subscription format
                    let sub_id = obj.remove("subscription_id").unwrap();
                    let node_idx = obj.remove("node_index").unwrap();
                    let mut node_ref_obj = serde_json::Map::new();
                    node_ref_obj.insert("type".into(), JsonValue::String("subscription".into()));
                    node_ref_obj.insert("subscription_id".into(), sub_id);
                    node_ref_obj.insert("node_index".into(), node_idx);
                    obj.insert("node_ref".into(), JsonValue::Object(node_ref_obj));
                    changed = true;
                } else if obj.contains_key("node_id") {
                    // Old manual format
                    let node_id = obj.remove("node_id").unwrap();
                    let mut node_ref_obj = serde_json::Map::new();
                    node_ref_obj.insert("type".into(), JsonValue::String("manual".into()));
                    node_ref_obj.insert("node_id".into(), node_id);
                    obj.insert("node_ref".into(), JsonValue::Object(node_ref_obj));
                    changed = true;
                }
            }
        }

        let Some(node_ref) = entry.get_mut("node_ref") else {
            return true;
        };
        match migrate_legacy_json_subscription_ref(node_ref, node_ids) {
            RefMigration::Unchanged => true,
            RefMigration::Updated => {
                changed = true;
                true
            }
            RefMigration::DropParent => {
                changed = true;
                false
            }
        }
    });

    changed || samples.len() != initial_len
}

fn migrate_legacy_json_subscription_ref(
    node_ref: &mut JsonValue,
    node_ids: &SubscriptionNodeMap,
) -> RefMigration {
    let Some(node_ref_obj) = node_ref.as_object_mut() else {
        return RefMigration::Unchanged;
    };
    if node_ref_obj.contains_key("node_id") {
        return RefMigration::Unchanged;
    }
    if node_ref_obj.get("type").and_then(JsonValue::as_str) != Some("subscription") {
        return RefMigration::Unchanged;
    }

    let Some(subscription_id) = node_ref_obj.get("subscription_id").and_then(json_uuid) else {
        return RefMigration::DropParent;
    };
    let Some(node_index) = node_ref_obj.get("node_index").and_then(json_usize) else {
        return RefMigration::DropParent;
    };
    let Some(node_id) = node_ids.get(&(subscription_id, node_index)).copied() else {
        return RefMigration::DropParent;
    };

    node_ref_obj.remove("node_index");
    node_ref_obj.insert("node_id".into(), JsonValue::String(node_id.to_string()));
    RefMigration::Updated
}

fn json_usize(value: &JsonValue) -> Option<usize> {
    value.as_u64().and_then(|value| usize::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::models::*;

    #[test]
    fn test_load_latency_snapshot_migrates_legacy_subscription_refs() {
        let (_tmp, paths) = super::super::test_paths();
        let mut subscription = Subscription::new_from_url("Migrated", "https://example.com/sub");
        subscription.nodes = vec![
            SubscriptionNode::new(ProxyNode::Vless(VlessConfig {
                address: "one.example.com".into(),
                port: 443,
                uuid: "node-1".into(),
                encryption: None,
                flow: None,
                transport: TransportSettings::Tcp,
                tls: None,
                remark: None,
            })),
            SubscriptionNode::new(ProxyNode::Vless(VlessConfig {
                address: "two.example.com".into(),
                port: 443,
                uuid: "node-2".into(),
                encryption: None,
                flow: None,
                transport: TransportSettings::Tcp,
                tls: None,
                remark: None,
            })),
        ];
        super::super::save_subscriptions(&paths, &[subscription.clone()]).unwrap();

        let raw = serde_json::json!({
            "samples": [
                {
                    "node_ref": {
                        "type": "subscription",
                        "subscription_id": subscription.id,
                        "node_index": 0
                    },
                    "sample": {
                        "latency_ms": 42,
                        "measured_at": "2025-01-01T00:00:00Z"
                    }
                },
                {
                    "node_ref": {
                        "type": "subscription",
                        "subscription_id": subscription.id,
                        "node_index": 99
                    },
                    "sample": {
                        "latency_ms": 99,
                        "measured_at": "2025-01-01T00:00:00Z"
                    }
                }
            ]
        });
        paths.ensure_dirs().unwrap();
        fs::write(
            paths.latency_snapshot_path(),
            serde_json::to_string_pretty(&raw).unwrap(),
        )
        .unwrap();

        let snapshot = load_latency_snapshot(&paths).unwrap();

        assert_eq!(snapshot.len(), 1);
        let node_ref = ConnectionNodeRef::Subscription {
            subscription_id: subscription.id,
            node_id: subscription.nodes[0].id,
        };
        assert!(snapshot.get(node_ref).is_some());
    }

    #[test]
    fn test_load_latency_snapshot_migrates_old_flat_format() {
        let (_tmp, paths) = super::super::test_paths();
        let mut subscription = Subscription::new_from_url("Old Format", "https://example.com/sub");
        subscription.nodes = vec![
            SubscriptionNode::new(ProxyNode::Vless(VlessConfig {
                address: "one.example.com".into(),
                port: 443,
                uuid: "node-1".into(),
                encryption: None,
                flow: None,
                transport: TransportSettings::Tcp,
                tls: None,
                remark: None,
            })),
            SubscriptionNode::new(ProxyNode::Vless(VlessConfig {
                address: "two.example.com".into(),
                port: 443,
                uuid: "node-2".into(),
                encryption: None,
                flow: None,
                transport: TransportSettings::Tcp,
                tls: None,
                remark: None,
            })),
        ];
        super::super::save_subscriptions(&paths, &[subscription.clone()]).unwrap();

        let manual_node_id = uuid::Uuid::new_v4();

        let raw = serde_json::json!({
            "samples": [
                {
                    "subscription_id": subscription.id,
                    "node_index": 0,
                    "sample": {
                        "latency_ms": 1224,
                        "measured_at": "2025-01-01T00:00:00Z"
                    }
                },
                {
                    "subscription_id": subscription.id,
                    "node_index": 99,
                    "sample": {
                        "latency_ms": 99,
                        "measured_at": "2025-01-01T00:00:00Z"
                    }
                },
                {
                    "node_id": manual_node_id,
                    "sample": {
                        "latency_ms": 55,
                        "measured_at": "2025-01-01T00:00:00Z"
                    }
                }
            ]
        });
        paths.ensure_dirs().unwrap();
        fs::write(
            paths.latency_snapshot_path(),
            serde_json::to_string_pretty(&raw).unwrap(),
        )
        .unwrap();

        let snapshot = load_latency_snapshot(&paths).unwrap();

        assert_eq!(snapshot.len(), 2);
        let node_ref = ConnectionNodeRef::Subscription {
            subscription_id: subscription.id,
            node_id: subscription.nodes[0].id,
        };
        assert!(snapshot.get(node_ref).is_some());
        let manual_ref = ConnectionNodeRef::Manual { node_id: manual_node_id };
        assert!(snapshot.get(manual_ref).is_some());
    }
}
