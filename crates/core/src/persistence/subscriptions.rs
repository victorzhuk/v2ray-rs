use serde_json::Value as JsonValue;
use uuid::Uuid;

use crate::fs::atomic_write;
use crate::models::Subscription;

use super::{AppPaths, PersistenceError, json_uuid, read_file};

pub fn save_subscriptions(
    paths: &AppPaths,
    subscriptions: &[Subscription],
) -> Result<(), PersistenceError> {
    paths.ensure_dirs()?;
    let json = serde_json::to_string_pretty(subscriptions)?;
    atomic_write(&paths.subscriptions_path(), json.as_bytes()).map_err(PersistenceError::Io)
}

pub fn load_subscriptions(paths: &AppPaths) -> Result<Vec<Subscription>, PersistenceError> {
    let path = paths.subscriptions_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = read_file(&path)?;
    let mut raw: JsonValue = serde_json::from_str(&contents)?;
    let migrated = migrate_subscription_node_ids(&mut raw);
    let subs: Vec<Subscription> = serde_json::from_value(raw)?;
    if migrated {
        save_subscriptions(paths, &subs)?;
    }
    Ok(subs)
}

pub fn add_subscription(
    paths: &AppPaths,
    subscription: Subscription,
) -> Result<(), PersistenceError> {
    let mut subs = load_subscriptions(paths)?;
    subs.push(subscription);
    save_subscriptions(paths, &subs)
}

pub fn get_subscription(
    paths: &AppPaths,
    id: &Uuid,
) -> Result<Option<Subscription>, PersistenceError> {
    let subs = load_subscriptions(paths)?;
    Ok(subs.into_iter().find(|s| &s.id == id))
}

pub fn update_subscription(
    paths: &AppPaths,
    subscription: Subscription,
) -> Result<bool, PersistenceError> {
    let mut subs = load_subscriptions(paths)?;
    match subs.iter_mut().find(|s| s.id == subscription.id) {
        Some(existing) => {
            *existing = subscription;
            save_subscriptions(paths, &subs)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

pub fn remove_subscription(paths: &AppPaths, id: &Uuid) -> Result<bool, PersistenceError> {
    let mut subs = load_subscriptions(paths)?;
    let initial_len = subs.len();
    subs.retain(|s| &s.id != id);
    if subs.len() < initial_len {
        save_subscriptions(paths, &subs)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn migrate_subscription_node_ids(raw: &mut JsonValue) -> bool {
    let Some(subscriptions) = raw.as_array_mut() else {
        return false;
    };

    let mut changed = false;
    for subscription in subscriptions {
        let Some(nodes) = subscription
            .get_mut("nodes")
            .and_then(JsonValue::as_array_mut)
        else {
            continue;
        };
        for node in nodes {
            if node.get("id").and_then(json_uuid).is_some() {
                continue;
            }
            let Some(node_obj) = node.as_object_mut() else {
                continue;
            };
            node_obj.insert("id".into(), JsonValue::String(Uuid::new_v4().to_string()));
            changed = true;
        }
    }

    changed
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_subscriptions_save_load_roundtrip() {
        let (_tmp, paths) = super::super::test_paths();
        let subs = vec![Subscription::new_from_url(
            "Test Sub",
            "https://example.com/sub",
        )];

        save_subscriptions(&paths, &subs).unwrap();
        let loaded = load_subscriptions(&paths).unwrap();

        assert_eq!(subs.len(), loaded.len());
        assert_eq!(subs[0].name, loaded[0].name);
    }

    #[test]
    fn test_load_subscriptions_missing_file() {
        let (_tmp, paths) = super::super::test_paths();
        paths.ensure_dirs().unwrap();
        let loaded = load_subscriptions(&paths).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_add_subscription() {
        let (_tmp, paths) = super::super::test_paths();
        let sub1 = Subscription::new_from_url("First", "https://example.com/1");
        let sub2 = Subscription::new_from_url("Second", "https://example.com/2");

        add_subscription(&paths, sub1.clone()).unwrap();
        add_subscription(&paths, sub2.clone()).unwrap();

        let loaded = load_subscriptions(&paths).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, sub1.id);
        assert_eq!(loaded[1].id, sub2.id);
    }

    #[test]
    fn test_get_subscription() {
        let (_tmp, paths) = super::super::test_paths();
        let sub = Subscription::new_from_url("Test", "https://example.com/sub");

        add_subscription(&paths, sub.clone()).unwrap();

        let found = get_subscription(&paths, &sub.id).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, sub.id);

        let not_found = get_subscription(&paths, &Uuid::new_v4()).unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_update_subscription() {
        let (_tmp, paths) = super::super::test_paths();
        let mut sub = Subscription::new_from_url("Original", "https://example.com/sub");

        add_subscription(&paths, sub.clone()).unwrap();

        sub.name = "Updated".into();
        let updated = update_subscription(&paths, sub.clone()).unwrap();
        assert!(updated);

        let loaded = get_subscription(&paths, &sub.id).unwrap().unwrap();
        assert_eq!(loaded.name, "Updated");
    }

    #[test]
    fn test_remove_subscription() {
        let (_tmp, paths) = super::super::test_paths();
        let sub1 = Subscription::new_from_url("First", "https://example.com/1");
        let sub2 = Subscription::new_from_url("Second", "https://example.com/2");

        add_subscription(&paths, sub1.clone()).unwrap();
        add_subscription(&paths, sub2.clone()).unwrap();

        let removed = remove_subscription(&paths, &sub1.id).unwrap();
        assert!(removed);

        let loaded = load_subscriptions(&paths).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, sub2.id);
    }

    #[test]
    fn test_remove_nonexistent() {
        let (_tmp, paths) = super::super::test_paths();
        let removed = remove_subscription(&paths, &Uuid::new_v4()).unwrap();
        assert!(!removed);
    }

    #[test]
    fn test_multiple_independent_subscriptions() {
        let (_tmp, paths) = super::super::test_paths();
        let sub1 = Subscription::new_from_url("URL1", "https://example.com/1");
        let sub2 = Subscription::new_from_url("URL2", "https://example.com/2");
        let sub3 = Subscription::new_from_file("File1", "/path/to/file");

        add_subscription(&paths, sub1.clone()).unwrap();
        add_subscription(&paths, sub2.clone()).unwrap();
        add_subscription(&paths, sub3.clone()).unwrap();

        let loaded = load_subscriptions(&paths).unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].source, sub1.source);
        assert_eq!(loaded[1].source, sub2.source);
        assert_eq!(loaded[2].source, sub3.source);
        assert!(loaded.iter().all(|s| s.nodes.is_empty()));
    }

    #[test]
    fn test_load_subscriptions_migrates_missing_node_ids() {
        let (_tmp, paths) = super::super::test_paths();
        let raw = serde_json::json!([
            {
                "id": Uuid::new_v4(),
                "name": "Migrated",
                "source": { "type": "url", "url": "https://example.com/sub" },
                "nodes": [
                    {
                        "node": {
                            "protocol": "vless",
                            "address": "example.com",
                            "port": 443,
                            "uuid": "test-uuid"
                        },
                        "enabled": true
                    }
                ],
                "last_updated": null,
                "auto_update_interval_secs": 86400,
                "enabled": true
            }
        ]);

        paths.ensure_dirs().unwrap();
        fs::write(
            paths.subscriptions_path(),
            serde_json::to_string_pretty(&raw).unwrap(),
        )
        .unwrap();

        let first = load_subscriptions(&paths).unwrap();
        let second = load_subscriptions(&paths).unwrap();

        assert_eq!(first.len(), 1);
        assert_eq!(first[0].nodes.len(), 1);
        assert_eq!(first[0].nodes[0].id, second[0].nodes[0].id);
    }
}
