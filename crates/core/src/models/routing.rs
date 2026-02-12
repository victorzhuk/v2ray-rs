use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::validation::{ValidationError, validate_rule_match};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingRule {
    pub id: Uuid,
    pub match_condition: RuleMatch,
    pub action: RuleAction,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleMatch {
    GeoIp { country_code: String },
    GeoSite { category: String },
    Domain { pattern: String },
    IpCidr { cidr: IpNet },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Proxy,
    Direct,
    Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingRuleSet {
    rules: Vec<RoutingRule>,
}

impl RoutingRuleSet {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn add(&mut self, rule: RoutingRule) {
        self.rules.push(rule);
    }

    pub fn remove(&mut self, id: &Uuid) -> Option<RoutingRule> {
        if let Some(pos) = self.rules.iter().position(|r| r.id == *id) {
            Some(self.rules.remove(pos))
        } else {
            None
        }
    }

    pub fn move_rule(&mut self, from: usize, to: usize) {
        if from < self.rules.len() && to < self.rules.len() {
            let rule = self.rules.remove(from);
            self.rules.insert(to, rule);
        }
    }

    pub fn rules(&self) -> &[RoutingRule] {
        &self.rules
    }

    pub fn rules_mut(&mut self) -> &mut Vec<RoutingRule> {
        &mut self.rules
    }

    pub fn enabled_rules(&self) -> impl Iterator<Item = &RoutingRule> {
        self.rules.iter().filter(|r| r.enabled)
    }

    pub fn apply_preset(&mut self, preset: &crate::models::presets::Preset) {
        for rule in preset.rules() {
            let already_exists = self
                .rules
                .iter()
                .any(|r| r.match_condition == rule.match_condition);
            if !already_exists {
                self.rules.push(rule);
            }
        }
    }

    pub fn add_validated(&mut self, rule: RoutingRule) -> Result<(), ValidationError> {
        validate_rule_match(&rule.match_condition)?;
        self.rules.push(rule);
        Ok(())
    }

    pub fn add_at(&mut self, index: usize, rule: RoutingRule) -> Result<(), ValidationError> {
        validate_rule_match(&rule.match_condition)?;
        if index > self.rules.len() {
            return Err(ValidationError::IndexOutOfBounds(index));
        }
        self.rules.insert(index, rule);
        Ok(())
    }

    pub fn edit_rule(
        &mut self,
        id: &Uuid,
        match_condition: Option<RuleMatch>,
        action: Option<RuleAction>,
    ) -> Result<bool, ValidationError> {
        if let Some(rule) = self.rules.iter_mut().find(|r| r.id == *id) {
            if let Some(new_match) = match_condition {
                validate_rule_match(&new_match)?;
                rule.match_condition = new_match;
            }
            if let Some(new_action) = action {
                rule.action = new_action;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl Default for RoutingRuleSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rule(country: &str, action: RuleAction) -> RoutingRule {
        RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: country.into(),
            },
            action,
            enabled: true,
        }
    }

    #[test]
    fn test_rule_set_ordering() {
        let mut set = RoutingRuleSet::new();
        let r1 = make_rule("RU", RuleAction::Direct);
        let r2 = make_rule("US", RuleAction::Proxy);
        let r3 = make_rule("CN", RuleAction::Block);

        set.add(r1.clone());
        set.add(r2.clone());
        set.add(r3.clone());

        assert_eq!(set.rules().len(), 3);
        assert_eq!(set.rules()[0].match_condition, r1.match_condition);
        assert_eq!(set.rules()[2].match_condition, r3.match_condition);
    }

    #[test]
    fn test_rule_set_move() {
        let mut set = RoutingRuleSet::new();
        let r1 = make_rule("RU", RuleAction::Direct);
        let r2 = make_rule("US", RuleAction::Proxy);
        let r3 = make_rule("CN", RuleAction::Block);

        let r3_match = r3.match_condition.clone();
        set.add(r1);
        set.add(r2);
        set.add(r3);

        set.move_rule(2, 0);
        assert_eq!(set.rules()[0].match_condition, r3_match);
    }

    #[test]
    fn test_rule_set_remove() {
        let mut set = RoutingRuleSet::new();
        let r1 = make_rule("RU", RuleAction::Direct);
        let id = r1.id;
        set.add(r1);
        assert_eq!(set.rules().len(), 1);

        let removed = set.remove(&id);
        assert!(removed.is_some());
        assert_eq!(set.rules().len(), 0);
    }

    #[test]
    fn test_enabled_rules_filter() {
        let mut set = RoutingRuleSet::new();
        let mut r1 = make_rule("RU", RuleAction::Direct);
        r1.enabled = false;
        let r2 = make_rule("US", RuleAction::Proxy);

        set.add(r1);
        set.add(r2);

        let enabled: Vec<_> = set.enabled_rules().collect();
        assert_eq!(enabled.len(), 1);
        assert_eq!(
            enabled[0].match_condition,
            RuleMatch::GeoIp {
                country_code: "US".into()
            }
        );
    }

    #[test]
    fn test_geoip_rule_serialization() {
        let rule = make_rule("RU", RuleAction::Direct);
        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: RoutingRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_domain_rule() {
        let rule = RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::Domain {
                pattern: "*.google.com".into(),
            },
            action: RuleAction::Proxy,
            enabled: true,
        };
        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: RoutingRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_ip_cidr_rule() {
        let rule = RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::IpCidr {
                cidr: "192.168.0.0/16".parse().unwrap(),
            },
            action: RuleAction::Direct,
            enabled: true,
        };
        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: RoutingRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_add_validated_success() {
        let mut set = RoutingRuleSet::new();
        let rule = RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: "US".to_string(),
            },
            action: RuleAction::Proxy,
            enabled: true,
        };

        let result = set.add_validated(rule.clone());
        assert!(result.is_ok());
        assert_eq!(set.rules().len(), 1);
        assert_eq!(set.rules()[0].id, rule.id);
    }

    #[test]
    fn test_add_validated_invalid_country() {
        let mut set = RoutingRuleSet::new();
        let rule = RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: "USA".to_string(),
            },
            action: RuleAction::Proxy,
            enabled: true,
        };

        let result = set.add_validated(rule);
        assert!(result.is_err());
        assert_eq!(set.rules().len(), 0);
    }

    #[test]
    fn test_add_validated_invalid_domain() {
        let mut set = RoutingRuleSet::new();
        let rule = RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::Domain {
                pattern: ".example.com".to_string(),
            },
            action: RuleAction::Proxy,
            enabled: true,
        };

        let result = set.add_validated(rule);
        assert!(result.is_err());
        assert_eq!(set.rules().len(), 0);
    }

    #[test]
    fn test_add_at_success() {
        let mut set = RoutingRuleSet::new();
        let r1 = make_rule("US", RuleAction::Proxy);
        let r2 = make_rule("CN", RuleAction::Direct);
        set.add(r1.clone());
        set.add(r2.clone());

        let r_middle = RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: "RU".to_string(),
            },
            action: RuleAction::Block,
            enabled: true,
        };

        let result = set.add_at(1, r_middle.clone());
        assert!(result.is_ok());
        assert_eq!(set.rules().len(), 3);
        assert_eq!(set.rules()[1].id, r_middle.id);
        assert_eq!(set.rules()[0].id, r1.id);
        assert_eq!(set.rules()[2].id, r2.id);
    }

    #[test]
    fn test_add_at_invalid() {
        let mut set = RoutingRuleSet::new();
        let rule = RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::GeoIp {
                country_code: "ZZ".to_string(),
            },
            action: RuleAction::Proxy,
            enabled: true,
        };

        let result = set.add_at(0, rule);
        assert!(result.is_err());
        assert_eq!(set.rules().len(), 0);
    }

    #[test]
    fn test_edit_rule_match_condition() {
        let mut set = RoutingRuleSet::new();
        let rule = make_rule("US", RuleAction::Proxy);
        let id = rule.id;
        set.add(rule);

        let new_match = RuleMatch::Domain {
            pattern: "example.com".to_string(),
        };

        let result = set.edit_rule(&id, Some(new_match.clone()), None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
        assert_eq!(set.rules()[0].match_condition, new_match);
        assert_eq!(set.rules()[0].action, RuleAction::Proxy);
    }

    #[test]
    fn test_edit_rule_action() {
        let mut set = RoutingRuleSet::new();
        let rule = make_rule("US", RuleAction::Proxy);
        let id = rule.id;
        let original_match = rule.match_condition.clone();
        set.add(rule);

        let result = set.edit_rule(&id, None, Some(RuleAction::Block));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
        assert_eq!(set.rules()[0].match_condition, original_match);
        assert_eq!(set.rules()[0].action, RuleAction::Block);
    }

    #[test]
    fn test_edit_rule_both() {
        let mut set = RoutingRuleSet::new();
        let rule = make_rule("US", RuleAction::Proxy);
        let id = rule.id;
        set.add(rule);

        let new_match = RuleMatch::GeoSite {
            category: "google".to_string(),
        };

        let result = set.edit_rule(&id, Some(new_match.clone()), Some(RuleAction::Direct));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
        assert_eq!(set.rules()[0].match_condition, new_match);
        assert_eq!(set.rules()[0].action, RuleAction::Direct);
    }

    #[test]
    fn test_edit_rule_not_found() {
        let mut set = RoutingRuleSet::new();
        let rule = make_rule("US", RuleAction::Proxy);
        set.add(rule);

        let random_id = Uuid::new_v4();
        let result = set.edit_rule(&random_id, None, Some(RuleAction::Block));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), false);
    }

    #[test]
    fn test_edit_rule_invalid_match() {
        let mut set = RoutingRuleSet::new();
        let rule = make_rule("US", RuleAction::Proxy);
        let id = rule.id;
        set.add(rule);

        let invalid_match = RuleMatch::Domain {
            pattern: ".invalid".to_string(),
        };

        let result = set.edit_rule(&id, Some(invalid_match), None);
        assert!(result.is_err());
    }
}
