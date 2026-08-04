use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{AppSettings, ConnectionNodeRef, DnsConfig, RoutingRule, Subscription};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportedProfile {
    pub rules: Vec<RoutingRule>,
    pub dns: Option<DnsConfig>,
    pub skipped: Vec<String>,
    pub imported_at: DateTime<Utc>,
}

/// Resolves the routing rules and settings to generate a config with for one
/// connection candidate. A subscription-owned node with an imported profile
/// still enabled uses the provider's rules and DNS instead of the app's
/// global ones; ports, `listen_address` and TUN always stay app-owned.
pub fn resolve_effective_config(
    node_ref: &ConnectionNodeRef,
    subscriptions: &[Subscription],
    global_rules: &[RoutingRule],
    settings: &AppSettings,
) -> (Vec<RoutingRule>, AppSettings) {
    if let ConnectionNodeRef::Subscription {
        subscription_id, ..
    } = node_ref
        && let Some(sub) = subscriptions.iter().find(|s| s.id == *subscription_id)
        && sub.use_imported_profile
        && let Some(profile) = &sub.imported_profile
    {
        let rules: Vec<RoutingRule> = profile
            .rules
            .iter()
            .filter(|r| r.enabled)
            .cloned()
            .collect();
        let mut effective = settings.clone();
        if let Some(dns) = &profile.dns {
            effective.dns = dns.clone();
        }
        return (rules, effective);
    }

    (global_rules.to_vec(), settings.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        ProxyNode, RuleAction, RuleMatch, SubscriptionNode, TransportSettings, VlessConfig,
    };
    use uuid::Uuid;

    fn vless_node() -> ProxyNode {
        ProxyNode::Vless(VlessConfig {
            address: "example.com".into(),
            port: 443,
            uuid: "test-uuid".into(),
            encryption: None,
            flow: None,
            transport: TransportSettings::Tcp,
            tls: None,
            remark: None,
        })
    }

    fn profile_rule(enabled: bool) -> RoutingRule {
        RoutingRule {
            id: Uuid::new_v4(),
            match_condition: RuleMatch::GeoSite {
                category: "google".into(),
            },
            action: RuleAction::Proxy,
            enabled,
            group: None,
        }
    }

    fn subscription_with_profile(use_profile: bool) -> Subscription {
        let mut sub = Subscription::new_from_url("Provider", "https://example.com/sub");
        sub.nodes = vec![SubscriptionNode::new(vless_node())];
        sub.use_imported_profile = use_profile;
        sub.imported_profile = Some(ImportedProfile {
            rules: vec![profile_rule(true), profile_rule(false)],
            dns: Some(DnsConfig {
                enabled: true,
                ..DnsConfig::default()
            }),
            skipped: vec![],
            imported_at: Utc::now(),
        });
        sub
    }

    #[test]
    fn uses_profile_rules_and_dns_when_enabled() {
        let sub = subscription_with_profile(true);
        let node_ref = ConnectionNodeRef::Subscription {
            subscription_id: sub.id,
            node_id: sub.nodes[0].id,
        };
        let settings = AppSettings::default();
        let global_rules = vec![profile_rule(true), profile_rule(true)];

        let (rules, effective) =
            resolve_effective_config(&node_ref, &[sub], &global_rules, &settings);

        assert_eq!(rules.len(), 1, "only the enabled profile rule survives");
        assert!(effective.dns.enabled);
    }

    #[test]
    fn falls_back_to_global_when_profile_disabled() {
        let sub = subscription_with_profile(false);
        let node_ref = ConnectionNodeRef::Subscription {
            subscription_id: sub.id,
            node_id: sub.nodes[0].id,
        };
        let settings = AppSettings::default();
        let global_rules = vec![profile_rule(true), profile_rule(true)];

        let (rules, effective) =
            resolve_effective_config(&node_ref, &[sub], &global_rules, &settings);

        assert_eq!(rules.len(), 2);
        assert_eq!(effective.dns, settings.dns);
    }

    #[test]
    fn falls_back_to_global_when_no_profile() {
        let mut sub = Subscription::new_from_url("Plain", "https://example.com/sub");
        sub.nodes = vec![SubscriptionNode::new(vless_node())];
        let node_ref = ConnectionNodeRef::Subscription {
            subscription_id: sub.id,
            node_id: sub.nodes[0].id,
        };
        let settings = AppSettings::default();
        let global_rules = vec![profile_rule(true)];

        let (rules, effective) =
            resolve_effective_config(&node_ref, &[sub], &global_rules, &settings);

        assert_eq!(rules.len(), 1);
        assert_eq!(effective.dns, settings.dns);
    }

    #[test]
    fn falls_back_to_global_for_manual_node() {
        let node_ref = ConnectionNodeRef::Manual {
            node_id: Uuid::new_v4(),
        };
        let settings = AppSettings::default();
        let global_rules = vec![profile_rule(true)];

        let (rules, effective) = resolve_effective_config(&node_ref, &[], &global_rules, &settings);

        assert_eq!(rules.len(), 1);
        assert_eq!(effective.dns, settings.dns);
    }
}
