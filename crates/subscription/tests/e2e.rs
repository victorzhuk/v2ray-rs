use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tempfile::TempDir;
use v2ray_rs_core::models::{ProxyNode, Subscription};
use v2ray_rs_core::persistence::{
    add_subscription, get_subscription, load_subscriptions, remove_subscription,
    update_subscription, AppPaths,
};
use v2ray_rs_subscription::fetch::{decode_subscription_content, fetch_from_file};
use v2ray_rs_subscription::parser::parse_subscription_uris;
use v2ray_rs_subscription::update::reconcile_with_counts;

#[test]
fn test_subscription_full_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let paths = AppPaths::from_paths(tmp.path().join("config"), tmp.path().join("data"));

    let vmess_json =
        r#"{"add":"vmess.test.com","port":"443","id":"test-uuid","ps":"VMess Node"}"#;
    let vmess_uri = format!("vmess://{}", STANDARD.encode(vmess_json));

    let vless_uri = "vless://550e8400-e29b-41d4-a716-446655440000@vless.test.com:443#VLESS%20Node";

    let ss_userinfo = "aes-256-gcm:secret";
    let ss_encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(ss_userinfo);
    let ss_uri = format!("ss://{}@ss.test.com:8388#SS%20Node", ss_encoded);

    let trojan_uri = "trojan://pass@trojan.test.com:443#Trojan%20Node";

    let subscription_content = format!("{}\n{}\n{}\n{}", vmess_uri, vless_uri, ss_uri, trojan_uri);
    let encoded_subscription = STANDARD.encode(&subscription_content);

    let sub_file_path = tmp.path().join("subscription.txt");
    std::fs::write(&sub_file_path, &encoded_subscription).unwrap();

    let mut sub = Subscription::new_from_file("Test Subscription", sub_file_path.to_str().unwrap());
    add_subscription(&paths, sub.clone()).unwrap();

    let loaded_subs = load_subscriptions(&paths).unwrap();
    assert_eq!(loaded_subs.len(), 1);
    assert_eq!(loaded_subs[0].name, "Test Subscription");
    assert_eq!(loaded_subs[0].nodes.len(), 0);

    let raw_content = fetch_from_file(sub_file_path.to_str().unwrap()).unwrap();
    let uris = decode_subscription_content(&raw_content);
    let import_result = parse_subscription_uris(&uris);

    assert_eq!(import_result.nodes.len(), 4);
    assert_eq!(import_result.errors.len(), 0);

    let protocols: Vec<_> = import_result
        .nodes
        .iter()
        .map(|n| match &n.node {
            ProxyNode::Vless(_) => "vless",
            ProxyNode::Vmess(_) => "vmess",
            ProxyNode::Shadowsocks(_) => "ss",
            ProxyNode::Trojan(_) => "trojan",
        })
        .collect();

    assert!(protocols.contains(&"vless"));
    assert!(protocols.contains(&"vmess"));
    assert!(protocols.contains(&"ss"));
    assert!(protocols.contains(&"trojan"));
    assert!(import_result.nodes.iter().all(|n| n.enabled));

    sub.nodes = import_result.nodes;
    update_subscription(&paths, sub.clone()).unwrap();

    let updated_sub = get_subscription(&paths, &sub.id).unwrap().unwrap();
    assert_eq!(updated_sub.nodes.len(), 4);
    assert!(updated_sub.nodes.iter().all(|n| n.enabled));

    let vless_uri2 = "vless://new-uuid@vless2.test.com:8443#New%20VLESS";
    let new_subscription_content = format!("{}\n{}\n{}", vless_uri, ss_uri, vless_uri2);
    let new_encoded = STANDARD.encode(&new_subscription_content);

    let sub_file_path2 = tmp.path().join("subscription_v2.txt");
    std::fs::write(&sub_file_path2, &new_encoded).unwrap();

    let raw_content2 = fetch_from_file(sub_file_path2.to_str().unwrap()).unwrap();
    let uris2 = decode_subscription_content(&raw_content2);
    let import_result2 = parse_subscription_uris(&uris2);

    let parsed_nodes: Vec<ProxyNode> = import_result2.nodes.iter().map(|n| n.node.clone()).collect();

    let (reconciled_nodes, update_result) = reconcile_with_counts(&sub.nodes, parsed_nodes);

    assert_eq!(update_result.added, 1);
    assert_eq!(update_result.removed, 2);
    assert_eq!(update_result.unchanged, 2);
    assert_eq!(reconciled_nodes.len(), 3);

    sub.nodes = reconciled_nodes;
    update_subscription(&paths, sub.clone()).unwrap();

    let updated_sub2 = get_subscription(&paths, &sub.id).unwrap().unwrap();
    assert_eq!(updated_sub2.nodes.len(), 3);

    sub.nodes[0].enabled = false;

    update_subscription(&paths, sub.clone()).unwrap();

    let raw_content3 = fetch_from_file(sub_file_path2.to_str().unwrap()).unwrap();
    let uris3 = decode_subscription_content(&raw_content3);
    let import_result3 = parse_subscription_uris(&uris3);

    let parsed_nodes3: Vec<ProxyNode> =
        import_result3.nodes.iter().map(|n| n.node.clone()).collect();

    let (reconciled_nodes3, _) = reconcile_with_counts(&sub.nodes, parsed_nodes3);

    assert_eq!(reconciled_nodes3.len(), 3);
    assert!(!reconciled_nodes3[0].enabled);
    assert!(reconciled_nodes3[1].enabled);
    assert!(reconciled_nodes3[2].enabled);

    let removed = remove_subscription(&paths, &sub.id).unwrap();
    assert!(removed);

    let final_subs = load_subscriptions(&paths).unwrap();
    assert_eq!(final_subs.len(), 0);
}
