use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{RoutingRule, RuleAction, RuleMatch};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub description: String,
    rules: Vec<PresetRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PresetRule {
    match_condition: RuleMatch,
    action: RuleAction,
}

impl Preset {
    pub fn from_rules(name: &str, description: &str, rules: &[RoutingRule]) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            rules: rules
                .iter()
                .map(|r| PresetRule {
                    match_condition: r.match_condition.clone(),
                    action: r.action,
                })
                .collect(),
        }
    }

    pub fn rules(&self) -> Vec<RoutingRule> {
        self.rules
            .iter()
            .map(|pr| RoutingRule {
                id: Uuid::new_v4(),
                match_condition: pr.match_condition.clone(),
                action: pr.action,
                enabled: true,
            })
            .collect()
    }
}

pub fn builtin_presets() -> Vec<Preset> {
    vec![
        Preset {
            name: "RU Direct".into(),
            description: "Route Russian traffic directly".into(),
            rules: vec![PresetRule {
                match_condition: RuleMatch::GeoIp {
                    country_code: "RU".into(),
                },
                action: RuleAction::Direct,
            }],
        },
        Preset {
            name: "CN Direct".into(),
            description: "Route Chinese traffic directly".into(),
            rules: vec![
                PresetRule {
                    match_condition: RuleMatch::GeoIp {
                        country_code: "CN".into(),
                    },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "cn".into(),
                    },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "geolocation-cn".into(),
                    },
                    action: RuleAction::Direct,
                },
            ],
        },
        Preset {
            name: "Block Ads".into(),
            description: "Block advertising domains".into(),
            rules: vec![PresetRule {
                match_condition: RuleMatch::GeoSite {
                    category: "category-ads-all".into(),
                },
                action: RuleAction::Block,
            }],
        },
        Preset {
            name: "Popular AI".into(),
            description: "Route AI services through proxy".into(),
            rules: vec![
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "openai".into(),
                    },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "anthropic".into(),
                    },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "google".into(),
                    },
                    action: RuleAction::Proxy,
                },
            ],
        },
        Preset {
            name: "Social Networks".into(),
            description: "Route social media through proxy".into(),
            rules: vec![
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "discord".into(),
                    },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "telegram".into(),
                    },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "whatsapp".into(),
                    },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "tiktok".into(),
                    },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "instagram".into(),
                    },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "twitter".into(),
                    },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "facebook".into(),
                    },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "youtube".into(),
                    },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "reddit".into(),
                    },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite {
                        category: "github".into(),
                    },
                    action: RuleAction::Proxy,
                },
            ],
        },
        Preset {
            name: "Bypass LAN".into(),
            description: "Route local network traffic directly".into(),
            rules: vec![
                PresetRule {
                    match_condition: RuleMatch::IpCidr {
                        cidr: "10.0.0.0/8".parse().unwrap(),
                    },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::IpCidr {
                        cidr: "172.16.0.0/12".parse().unwrap(),
                    },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::IpCidr {
                        cidr: "192.168.0.0/16".parse().unwrap(),
                    },
                    action: RuleAction::Direct,
                },
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_presets_count() {
        let presets = builtin_presets();
        assert_eq!(presets.len(), 6);
    }

    #[test]
    fn test_preset_generates_unique_uuids() {
        let presets = builtin_presets();
        let preset = &presets[0];

        let rules1 = preset.rules();
        let rules2 = preset.rules();

        assert_eq!(rules1.len(), rules2.len());
        for (r1, r2) in rules1.iter().zip(rules2.iter()) {
            assert_ne!(r1.id, r2.id);
            assert_eq!(r1.match_condition, r2.match_condition);
            assert_eq!(r1.action, r2.action);
        }
    }

    #[test]
    fn test_apply_preset() {
        use super::super::RoutingRuleSet;

        let mut rule_set = RoutingRuleSet::new();
        let presets = builtin_presets();
        let preset = &presets[0];

        rule_set.apply_preset(preset);
        assert_eq!(rule_set.rules().len(), 1);

        rule_set.apply_preset(preset);
        assert_eq!(rule_set.rules().len(), 1, "duplicates should be skipped");

        let ids: Vec<_> = rule_set.rules().iter().map(|r| r.id).collect();
        let unique_ids: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique_ids.len());
    }

    #[test]
    fn test_preset_rules_are_enabled() {
        let presets = builtin_presets();
        let rules = presets[0].rules();
        assert!(rules.iter().all(|r| r.enabled));
    }
}
