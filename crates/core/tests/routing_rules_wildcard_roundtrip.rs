#![cfg(feature = "test-utils")]

use uuid::Uuid;
use v2ray_rs_core::models::{RoutingRule, RoutingRuleSet, RuleAction, RuleMatch};
use v2ray_rs_core::persistence::{AppPaths, load_routing_rules, save_routing_rules};
use v2ray_rs_core::profile::AppProfile;

fn wildcard_set() -> RoutingRuleSet {
    let mut set = RoutingRuleSet::new();
    set.add(RoutingRule {
        id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        match_condition: RuleMatch::DomainKeyword {
            keyword: "*.ru".into(),
        },
        action: RuleAction::Direct,
        enabled: true,
        group: None,
        via_node: None,
    });
    set
}

#[test]
fn stored_wildcard_keyword_rule_loads_and_roundtrips() {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
    paths.ensure_dirs().unwrap();

    let original = wildcard_set();

    save_routing_rules(&paths, &original).unwrap();
    let loaded = load_routing_rules(&paths).unwrap();
    assert_eq!(loaded, original);
    assert_eq!(
        loaded.rules()[0].match_condition,
        RuleMatch::DomainKeyword {
            keyword: "*.ru".into()
        }
    );

    save_routing_rules(&paths, &loaded).unwrap();
    let reloaded = load_routing_rules(&paths).unwrap();
    assert_eq!(reloaded, original);
}

#[test]
fn stored_wildcard_json_loads_unchanged() {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
    paths.ensure_dirs().unwrap();

    let json = r#"{"rules":[{"id":"00000000-0000-0000-0000-000000000001","match_condition":{"type":"domain_keyword","keyword":"*.ru"},"action":"direct","enabled":true}]}"#;
    let set: RoutingRuleSet = serde_json::from_str(json).unwrap();
    save_routing_rules(&paths, &set).unwrap();
    let loaded = load_routing_rules(&paths).unwrap();
    assert_eq!(loaded, set);
    assert_eq!(
        loaded.rules()[0].match_condition,
        RuleMatch::DomainKeyword {
            keyword: "*.ru".into()
        }
    );
}

#[test]
fn missing_file_loads_empty() {
    let tmp = tempfile::TempDir::new().unwrap();
    let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
    paths.ensure_dirs().unwrap();
    let loaded = load_routing_rules(&paths).unwrap();
    assert!(loaded.rules().is_empty());
}
