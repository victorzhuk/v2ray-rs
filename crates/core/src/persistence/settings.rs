use std::collections::HashMap;

use toml::Value as TomlValue;
use uuid::Uuid;

use crate::fs::atomic_write;
use crate::models::AppSettings;

use super::{AppPaths, PersistenceError, RefMigration, read_file};

pub fn save_settings(paths: &AppPaths, settings: &AppSettings) -> Result<(), PersistenceError> {
    paths.ensure_dirs()?;
    let toml_str = toml::to_string_pretty(settings)?;
    atomic_write(&paths.settings_path(), toml_str.as_bytes()).map_err(PersistenceError::Io)
}

pub fn load_settings(paths: &AppPaths) -> Result<AppSettings, PersistenceError> {
    let path = paths.settings_path();
    if !path.exists() {
        return Ok(AppSettings::default());
    }
    let contents = read_file(&path)?;
    let mut raw: TomlValue = match toml::from_str(&contents) {
        Ok(raw) => raw,
        Err(e) => return Err(PersistenceError::CorruptConfig(e.to_string())),
    };
    let node_ids = load_subscription_node_map(paths);
    let migrated = migrate_settings_legacy_refs(&mut raw, &node_ids);
    match raw.try_into() {
        Ok(settings) => {
            if migrated {
                save_settings(paths, &settings)?;
            }
            Ok(settings)
        }
        Err(e) => Err(PersistenceError::CorruptConfig(e.to_string())),
    }
}

pub fn load_settings_or_default(paths: &AppPaths) -> AppSettings {
    match load_settings(paths) {
        Ok(s) => s,
        Err(PersistenceError::CorruptConfig(msg)) => {
            log::warn!("{msg}; using default settings");
            AppSettings::default()
        }
        Err(e) => {
            log::warn!("failed to load settings: {e}; using defaults");
            AppSettings::default()
        }
    }
}

pub(super) type SubscriptionNodeMap = HashMap<(Uuid, usize), Uuid>;

pub(super) fn load_subscription_node_map(paths: &AppPaths) -> SubscriptionNodeMap {
    super::load_subscriptions(paths)
        .map(|subscriptions| {
            subscriptions
                .into_iter()
                .flat_map(|subscription| {
                    subscription
                        .nodes
                        .into_iter()
                        .enumerate()
                        .map(move |(idx, node)| ((subscription.id, idx), node.id))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn migrate_settings_legacy_refs(raw: &mut TomlValue, node_ids: &SubscriptionNodeMap) -> bool {
    let Some(root) = raw.as_table_mut() else {
        return false;
    };
    let Some(last_success) = root.get_mut("last_success") else {
        return false;
    };
    let Some(last_success_table) = last_success.as_table_mut() else {
        return false;
    };
    let Some(node_ref) = last_success_table.get_mut("node_ref") else {
        return false;
    };

    match migrate_legacy_toml_subscription_ref(node_ref, node_ids) {
        RefMigration::Unchanged => false,
        RefMigration::Updated => true,
        RefMigration::DropParent => {
            root.remove("last_success");
            true
        }
    }
}

fn migrate_legacy_toml_subscription_ref(
    node_ref: &mut TomlValue,
    node_ids: &SubscriptionNodeMap,
) -> RefMigration {
    let Some(node_ref_table) = node_ref.as_table_mut() else {
        return RefMigration::Unchanged;
    };
    if node_ref_table.contains_key("node_id") {
        return RefMigration::Unchanged;
    }
    if node_ref_table.get("type").and_then(TomlValue::as_str) != Some("subscription") {
        return RefMigration::Unchanged;
    }

    let Some(subscription_id) = node_ref_table.get("subscription_id").and_then(toml_uuid) else {
        return RefMigration::DropParent;
    };
    let Some(node_index) = node_ref_table.get("node_index").and_then(toml_usize) else {
        return RefMigration::DropParent;
    };
    let Some(node_id) = node_ids.get(&(subscription_id, node_index)).copied() else {
        return RefMigration::DropParent;
    };

    node_ref_table.remove("node_index");
    node_ref_table.insert("node_id".into(), TomlValue::String(node_id.to_string()));
    RefMigration::Updated
}

fn toml_uuid(value: &TomlValue) -> Option<Uuid> {
    value.as_str().and_then(|value| Uuid::parse_str(value).ok())
}

fn toml_usize(value: &TomlValue) -> Option<usize> {
    value
        .as_integer()
        .and_then(|value| usize::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::models::*;

    #[test]
    fn test_settings_save_load_roundtrip() {
        let (_tmp, paths) = super::super::test_paths();
        let settings = AppSettings {
            socks_port: 9999,
            language: Language::Russian,
            ..AppSettings::default()
        };

        save_settings(&paths, &settings).unwrap();
        let loaded = load_settings(&paths).unwrap();
        assert_eq!(settings, loaded);
    }

    #[test]
    fn test_load_settings_missing_file_returns_default() {
        let (_tmp, paths) = super::super::test_paths();
        paths.ensure_dirs().unwrap();
        let loaded = load_settings(&paths).unwrap();
        assert_eq!(loaded, AppSettings::default());
    }

    #[test]
    fn test_corrupt_config_falls_back() {
        let (_tmp, paths) = super::super::test_paths();
        paths.ensure_dirs().unwrap();
        fs::write(paths.settings_path(), "invalid {{{{toml").unwrap();

        let loaded = load_settings_or_default(&paths);
        assert_eq!(loaded, AppSettings::default());
    }

    #[test]
    fn test_atomic_write_does_not_corrupt_on_overwrite() {
        let (_tmp, paths) = super::super::test_paths();
        let settings1 = AppSettings::default();
        save_settings(&paths, &settings1).unwrap();

        let settings2 = AppSettings {
            socks_port: 2222,
            ..AppSettings::default()
        };
        save_settings(&paths, &settings2).unwrap();

        let loaded = load_settings(&paths).unwrap();
        assert_eq!(loaded.socks_port, 2222);
    }

    #[test]
    fn test_load_settings_migrates_legacy_last_success_subscription_ref() {
        let (_tmp, paths) = super::super::test_paths();
        let mut subscription =
            crate::models::Subscription::new_from_url("Migrated", "https://example.com/sub");
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
        let expected_node_id = subscription.nodes[1].id;
        super::super::save_subscriptions(&paths, &[subscription.clone()]).unwrap();

        let settings = format!(
            "version = 1\nsocks_port = 1080\nhttp_port = 1081\nauto_update_subscriptions = true\nsubscription_update_interval_secs = 86400\nauto_update_geodata = true\ngeodata_update_interval_secs = 604800\nlanguage = \"english\"\nminimize_to_tray = true\nnotifications_enabled = true\nonboarding_complete = false\nauto_resolve_strategy = \"last-successful\"\n[backend]\nbackend_type = \"xray\"\n[dns]\nenabled = false\n[last_success]\nconnected_at = 2025-01-01T00:00:00Z\n[last_success.node_ref]\ntype = \"subscription\"\nsubscription_id = \"{}\"\nnode_index = 1\n",
            subscription.id
        );
        paths.ensure_dirs().unwrap();
        fs::write(paths.settings_path(), settings).unwrap();

        let loaded = load_settings(&paths).unwrap();

        assert_eq!(
            loaded.last_success,
            Some(LastSuccessMetadata {
                node_ref: ConnectionNodeRef::Subscription {
                    subscription_id: subscription.id,
                    node_id: expected_node_id,
                },
                connected_at: "2025-01-01T00:00:00Z"
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .unwrap(),
            })
        );
    }
}
