use std::sync::Once;

use v2ray_rs_core::models::Subscription;
use v2ray_rs_core::persistence::{load_subscriptions, save_subscriptions, AppPaths};
use v2ray_rs_core::profile::AppProfile;
use v2ray_rs_subscription::{
    SubscriptionError, SubscriptionService, UpdateError, MAX_SUBSCRIPTION_NODES,
};

fn install_crypto_provider() {
    static RUSTLS_PROVIDER: Once = Once::new();
    RUSTLS_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn write_uri_list(dir: &std::path::Path, name: &str, count: usize) -> String {
    let body: String = (0..count)
        .map(|i| format!("vless://uuid{i}@node{i}.example.com:443#N{i}\n"))
        .collect();
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    path.to_string_lossy().into_owned()
}

fn bundle_element(i: usize) -> serde_json::Value {
    serde_json::json!({
        "remarks": format!("N{i}"),
        "outbounds": [{
            "protocol": "vless",
            "settings": {
                "vnext": [{
                    "address": format!("node{i}.example.com"),
                    "port": 443,
                    "users": [{"id": "fixture-uuid", "encryption": "none"}]
                }]
            }
        }]
    })
}

fn write_bundle(dir: &std::path::Path, name: &str, count: usize) -> String {
    let elements: Vec<serde_json::Value> = (0..count).map(bundle_element).collect();
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_string(&elements).unwrap()).unwrap();
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn uri_list_at_node_cap_is_accepted() {
    install_crypto_provider();
    let dir = tempfile::tempdir().unwrap();
    let service = SubscriptionService::new();
    let sub = Subscription::new_from_file(
        "AtCap",
        write_uri_list(dir.path(), "at-cap.txt", MAX_SUBSCRIPTION_NODES),
    );

    let (sub, _result) = service.refresh(sub).await.unwrap();

    assert_eq!(sub.nodes.len(), MAX_SUBSCRIPTION_NODES);
}

#[tokio::test]
async fn uri_list_over_node_cap_preserves_previous_nodes() {
    install_crypto_provider();
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_profile_in(AppProfile::Test, dir.path());
    let service = SubscriptionService::new();

    let source_path = write_uri_list(dir.path(), "sub.txt", 2);
    let (sub, _result) = service
        .refresh(Subscription::new_from_file("Capped", source_path.clone()))
        .await
        .unwrap();
    assert_eq!(sub.nodes.len(), 2);
    let seeded = sub.clone();
    save_subscriptions(&paths, std::slice::from_ref(&sub)).unwrap();

    // Overwrite the SAME source file and refresh the SAME subscription: the
    // refusal must leave the previously stored record untouched, because a
    // caller only persists on success.
    write_uri_list(dir.path(), "sub.txt", MAX_SUBSCRIPTION_NODES + 1);
    let err = service.refresh(sub).await.unwrap_err();

    match err {
        SubscriptionError::Update(UpdateError::TooManyNodes { parsed }) => {
            assert_eq!(parsed, MAX_SUBSCRIPTION_NODES + 1);
            let msg = SubscriptionError::Update(UpdateError::TooManyNodes { parsed }).to_string();
            assert!(msg.contains("4096"), "limit must be named: {msg}");
        }
        other => panic!("expected node-limit refusal, got {other:?}"),
    }

    let stored = load_subscriptions(&paths).unwrap();
    assert_eq!(
        stored.len(),
        1,
        "failed refresh must not add or drop records"
    );
    assert_eq!(stored[0].id, seeded.id);
    assert_eq!(stored[0].nodes, seeded.nodes);
    assert_eq!(stored[0].last_updated, seeded.last_updated);
}

#[tokio::test]
async fn json_bundle_over_node_cap_preserves_previous_nodes() {
    install_crypto_provider();
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_profile_in(AppProfile::Test, dir.path());
    let service = SubscriptionService::new();

    let source_path = write_bundle(dir.path(), "bundle.json", 2);
    let (sub, _result) = service
        .refresh(Subscription::new_from_file("Bundle", source_path.clone()))
        .await
        .unwrap();
    assert_eq!(sub.nodes.len(), 2);
    let seeded = sub.clone();
    save_subscriptions(&paths, std::slice::from_ref(&sub)).unwrap();

    write_bundle(dir.path(), "bundle.json", MAX_SUBSCRIPTION_NODES + 1);
    let err = service.refresh(sub).await.unwrap_err();

    match err {
        SubscriptionError::Update(UpdateError::TooManyNodes { parsed }) => {
            assert_eq!(parsed, MAX_SUBSCRIPTION_NODES + 1);
            let msg = SubscriptionError::Update(UpdateError::TooManyNodes { parsed }).to_string();
            assert!(msg.contains("4096"), "limit must be named: {msg}");
        }
        other => panic!("expected node-limit refusal, got {other:?}"),
    }

    let stored = load_subscriptions(&paths).unwrap();
    assert_eq!(
        stored.len(),
        1,
        "failed refresh must not add or drop records"
    );
    assert_eq!(stored[0].id, seeded.id);
    assert_eq!(stored[0].nodes, seeded.nodes);
    assert_eq!(stored[0].last_updated, seeded.last_updated);
}

#[tokio::test]
async fn json_bundle_at_node_cap_is_accepted() {
    install_crypto_provider();
    let dir = tempfile::tempdir().unwrap();
    let service = SubscriptionService::new();

    let at_cap_path = write_bundle(dir.path(), "at-cap.json", MAX_SUBSCRIPTION_NODES);
    let sub = Subscription::new_from_file("Bundle", at_cap_path);

    let (sub, _result) = service.refresh(sub).await.unwrap();

    assert_eq!(sub.nodes.len(), MAX_SUBSCRIPTION_NODES);
    assert!(sub.imported_profile.is_some());
}
