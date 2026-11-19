#![cfg(feature = "test-utils")]

use chrono::Utc;
use v2ray_rs_core::models::{
    DnsConfig, DnsProtocol, DnsRule, DnsRuleMatch, DnsServerConfig, ImportedProfile, Subscription,
};
use v2ray_rs_core::persistence::{AppPaths, load_subscriptions, save_subscriptions};
use v2ray_rs_core::profile::AppProfile;

fn subscription_with_suffix(suffix: &str) -> Subscription {
    let mut sub = Subscription::new_from_url("Provider", "https://example.com/sub");
    sub.use_imported_profile = true;
    sub.imported_profile = Some(ImportedProfile {
        rules: Vec::new(),
        dns: Some(DnsConfig {
            enabled: true,
            servers: vec![DnsServerConfig {
                tag: "domestic".to_string(),
                protocol: DnsProtocol::Udp,
                address: "77.88.8.8".to_string(),
                port: None,
                detour: None,
            }],
            rules: vec![DnsRule {
                match_condition: DnsRuleMatch::DomainSuffix {
                    suffix: suffix.to_string(),
                },
                server_tag: "domestic".to_string(),
            }],
            ..Default::default()
        }),
        skipped: Vec::new(),
        imported_at: Utc::now(),
    });
    sub
}

fn stored_suffix(paths: &AppPaths) -> String {
    let contents = std::fs::read_to_string(paths.subscriptions_path()).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&contents).unwrap();
    raw[0]["imported_profile"]["dns"]["rules"][0]["match_condition"]["suffix"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn load_heals_leading_dot_dns_suffix_on_disk() {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
    paths.ensure_dirs().unwrap();

    save_subscriptions(&paths, &[subscription_with_suffix(".ru")]).unwrap();
    assert_eq!(stored_suffix(&paths), ".ru");

    let loaded = load_subscriptions(&paths).unwrap();
    let loaded_suffix = match &loaded[0]
        .imported_profile
        .as_ref()
        .unwrap()
        .dns
        .as_ref()
        .unwrap()
        .rules[0]
        .match_condition
    {
        DnsRuleMatch::DomainSuffix { suffix } => suffix.clone(),
        other => panic!("unexpected match condition: {other:?}"),
    };
    assert_eq!(loaded_suffix, "ru");
    assert_eq!(stored_suffix(&paths), "ru");

    let reloaded = load_subscriptions(&paths).unwrap();
    assert_eq!(reloaded, loaded);
}

#[test]
fn load_keeps_canonical_suffixes_unchanged() {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
    paths.ensure_dirs().unwrap();

    save_subscriptions(
        &paths,
        &[
            subscription_with_suffix("ru"),
            subscription_with_suffix("*.ru"),
        ],
    )
    .unwrap();

    let loaded = load_subscriptions(&paths).unwrap();
    let suffixes: Vec<String> = loaded
        .iter()
        .map(|sub| {
            match &sub
                .imported_profile
                .as_ref()
                .unwrap()
                .dns
                .as_ref()
                .unwrap()
                .rules[0]
                .match_condition
            {
                DnsRuleMatch::DomainSuffix { suffix } => suffix.clone(),
                other => panic!("unexpected match condition: {other:?}"),
            }
        })
        .collect();
    assert_eq!(suffixes, ["ru", "*.ru"]);
    assert_eq!(stored_suffix(&paths), "ru");
}
