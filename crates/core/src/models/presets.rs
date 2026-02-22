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
                group: Some(self.name.clone()),
            })
            .collect()
    }
}

pub fn builtin_presets() -> Vec<Preset> {
    vec![
        Preset {
            name: "RU Bypass".into(),
            description: "Route Russian and private traffic directly".into(),
            rules: vec![
                PresetRule {
                    match_condition: RuleMatch::GeoIp { country_code: "RU".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::IpCidr { cidr: "10.0.0.0/8".parse().unwrap() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::IpCidr { cidr: "172.16.0.0/12".parse().unwrap() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::IpCidr { cidr: "192.168.0.0/16".parse().unwrap() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::IpCidr { cidr: "169.254.0.0/16".parse().unwrap() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::IpCidr { cidr: "224.0.0.0/4".parse().unwrap() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::IpCidr { cidr: "255.255.255.255/32".parse().unwrap() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "category-ru".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "category-gov-ru".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "category-media-ru".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "category-retail-ru".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "mailru".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "mailru-group".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "category-entertainment-ru".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "category-ecommerce-ru".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "rutube".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "avito".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "kaspersky".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "yandex".into() },
                    action: RuleAction::Direct,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "category-doh".into() },
                    action: RuleAction::Direct,
                },
            ],
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
            name: "Proxy Popular".into(),
            description: "Route popular AI, social, and streaming services through proxy".into(),
            rules: vec![
                PresetRule {
                    match_condition: RuleMatch::GeoIp { country_code: "FACEBOOK".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoIp { country_code: "GOOGLE".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoIp { country_code: "NETFLIX".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoIp { country_code: "TELEGRAM".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoIp { country_code: "TWITTER".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "amazon".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "anthropic".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "aws".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "azure".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "aws-cn".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "category-ai-!cn".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "category-ai-chat-!cn".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "deezer".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "duckduckgo".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "facebook".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "f-droid".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "discord".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "telegram".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "whatsapp".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "tiktok".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "instagram".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "twitter".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "youtube".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "reddit".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "github".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "openai".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "google".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "netflix".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "spotify".into() },
                    action: RuleAction::Proxy,
                },
                PresetRule {
                    match_condition: RuleMatch::GeoSite { category: "stackoverflow".into() },
                    action: RuleAction::Proxy,
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
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_presets_count() {
        let presets = builtin_presets();
        assert_eq!(presets.len(), 4);
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
        let rule_count = preset.rules().len();

        rule_set.apply_preset(preset);
        assert_eq!(rule_set.rules().len(), rule_count);

        rule_set.apply_preset(preset);
        assert_eq!(
            rule_set.rules().len(),
            rule_count,
            "duplicates should be skipped"
        );

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
