use crate::fs::atomic_write;
use crate::models::RoutingRuleSet;

use super::{AppPaths, PersistenceError, read_file};

pub fn save_routing_rules(
    paths: &AppPaths,
    rules: &RoutingRuleSet,
) -> Result<(), PersistenceError> {
    paths.ensure_dirs()?;
    let json = serde_json::to_string_pretty(rules)?;
    atomic_write(&paths.routing_rules_path(), json.as_bytes()).map_err(PersistenceError::Io)
}

pub fn load_routing_rules(paths: &AppPaths) -> Result<RoutingRuleSet, PersistenceError> {
    let path = paths.routing_rules_path();
    if !path.exists() {
        return Ok(RoutingRuleSet::new());
    }
    let contents = read_file(&path)?;
    let rules: RoutingRuleSet = serde_json::from_str(&contents)?;
    Ok(rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use uuid::Uuid;

    #[test]
    fn test_routing_rules_save_load_roundtrip() {
        let (_tmp, paths) = super::super::test_paths();
        let mut rules = RoutingRuleSet::new();
        rules.add(RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: "RU".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
            group: None,
            via_node: None,
        });

        save_routing_rules(&paths, &rules).unwrap();
        let loaded = load_routing_rules(&paths).unwrap();

        assert_eq!(rules.rules().len(), loaded.rules().len());
        assert_eq!(
            rules.rules()[0].match_condition,
            loaded.rules()[0].match_condition
        );
    }

    #[test]
    fn test_load_routing_rules_missing_file() {
        let (_tmp, paths) = super::super::test_paths();
        paths.ensure_dirs().unwrap();
        let loaded = load_routing_rules(&paths).unwrap();
        assert!(loaded.rules().is_empty());
    }
}
