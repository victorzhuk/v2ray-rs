use uuid::Uuid;

use crate::models::{Preset, RoutingRule, RoutingRuleSet, RuleAction, RuleMatch, ValidationError};
use crate::persistence::{self, AppPaths, PersistenceError};

#[derive(Debug, thiserror::Error)]
pub enum RoutingManagerError {
    #[error("validation: {0}")]
    Validation(#[from] ValidationError),
    #[error("persistence: {0}")]
    Persistence(#[from] PersistenceError),
}

pub struct RoutingManager {
    rules: RoutingRuleSet,
    paths: AppPaths,
}

impl RoutingManager {
    pub fn load(paths: AppPaths) -> Result<Self, PersistenceError> {
        let rules = persistence::load_routing_rules(&paths)?;
        Ok(Self { rules, paths })
    }

    pub fn rules(&self) -> &RoutingRuleSet {
        &self.rules
    }

    pub fn add_rule(&mut self, rule: RoutingRule) -> Result<(), RoutingManagerError> {
        self.rules.add_validated(rule)?;
        self.persist()?;
        Ok(())
    }

    pub fn add_rule_at(
        &mut self,
        index: usize,
        rule: RoutingRule,
    ) -> Result<(), RoutingManagerError> {
        self.rules.add_at(index, rule)?;
        self.persist()?;
        Ok(())
    }

    pub fn edit_rule(
        &mut self,
        id: &Uuid,
        match_condition: Option<RuleMatch>,
        action: Option<RuleAction>,
    ) -> Result<bool, RoutingManagerError> {
        let found = self.rules.edit_rule(id, match_condition, action)?;
        if found {
            self.persist()?;
        }
        Ok(found)
    }

    pub fn delete_rule(&mut self, id: &Uuid) -> Result<bool, RoutingManagerError> {
        let removed = self.rules.remove(id).is_some();
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    pub fn reorder_rule(&mut self, from: usize, to: usize) -> Result<(), RoutingManagerError> {
        if self.rules.move_rule(from, to) {
            self.persist()?;
        }
        Ok(())
    }

    pub fn apply_preset(&mut self, preset: &Preset) -> Result<(), RoutingManagerError> {
        self.rules.apply_preset(preset);
        self.persist()?;
        Ok(())
    }

    pub fn replace_rules(&mut self, rules: RoutingRuleSet) -> Result<(), RoutingManagerError> {
        for rule in rules.rules() {
            crate::models::validate_rule_match(&rule.match_condition)?;
        }
        self.rules = rules;
        self.persist()?;
        Ok(())
    }

    fn persist(&self) -> Result<(), PersistenceError> {
        persistence::save_routing_rules(&self.paths, &self.rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use crate::profile::AppProfile;
    use tempfile::TempDir;

    fn setup() -> (TempDir, RoutingManager) {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        let manager = RoutingManager::load(paths).unwrap();
        (tmp, manager)
    }

    fn geoip_rule(country: &str, action: RuleAction) -> RoutingRule {
        RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: country.into(),
            },
            action,
            enabled: true,
            group: None,
        }
    }

    #[test]
    fn test_add_and_persist() {
        let (tmp, mut mgr) = setup();
        mgr.add_rule(geoip_rule("RU", RuleAction::Direct)).unwrap();

        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        let loaded = persistence::load_routing_rules(&paths).unwrap();
        assert_eq!(loaded.rules().len(), 1);
    }

    #[test]
    fn test_add_invalid_rejects() {
        let (_tmp, mut mgr) = setup();
        let rule = RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: "1Z".into(),
            },
            action: RuleAction::Direct,
            enabled: true,
            group: None,
        };
        assert!(mgr.add_rule(rule).is_err());
        assert!(mgr.rules().rules().is_empty());
    }

    #[test]
    fn test_delete_and_persist() {
        let (tmp, mut mgr) = setup();
        let rule = geoip_rule("US", RuleAction::Proxy);
        let id = rule.id;
        mgr.add_rule(rule).unwrap();

        assert!(mgr.delete_rule(&id).unwrap());

        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        let loaded = persistence::load_routing_rules(&paths).unwrap();
        assert!(loaded.rules().is_empty());
    }

    #[test]
    fn test_edit_and_persist() {
        let (tmp, mut mgr) = setup();
        let rule = geoip_rule("US", RuleAction::Proxy);
        let id = rule.id;
        mgr.add_rule(rule).unwrap();

        mgr.edit_rule(&id, None, Some(RuleAction::Block)).unwrap();

        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        let loaded = persistence::load_routing_rules(&paths).unwrap();
        assert_eq!(loaded.rules()[0].action, RuleAction::Block);
    }

    #[test]
    fn test_reorder_and_persist() {
        let (_tmp, mut mgr) = setup();
        let r1 = geoip_rule("US", RuleAction::Proxy);
        let r2 = geoip_rule("RU", RuleAction::Direct);
        let r2_id = r2.id;
        mgr.add_rule(r1).unwrap();
        mgr.add_rule(r2).unwrap();

        mgr.reorder_rule(1, 0).unwrap();
        assert_eq!(mgr.rules().rules()[0].id, r2_id);
    }

    #[test]
    fn test_apply_preset_and_persist() {
        let (tmp, mut mgr) = setup();
        let presets = builtin_presets();
        let preset = &presets[0];
        let expected = preset.rules().len();
        mgr.apply_preset(preset).unwrap();

        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        let loaded = persistence::load_routing_rules(&paths).unwrap();
        assert_eq!(loaded.rules().len(), expected);
    }
}
