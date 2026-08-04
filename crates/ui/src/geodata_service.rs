use std::time::Duration;

use v2ray_rs_core::models::{
    AppSettings, BackendType, RoutingRule, RoutingRuleSet, RuleMatch, Subscription,
};
use v2ray_rs_core::persistence::AppPaths;

#[derive(Clone)]
pub struct GeodataRefreshConfig {
    pub paths: AppPaths,
    pub backend_type: BackendType,
    pub enabled: bool,
    pub interval_secs: u64,
    pub singbox_rule_set_tags: Vec<String>,
}

impl GeodataRefreshConfig {
    pub fn from_settings(
        paths: &AppPaths,
        settings: &AppSettings,
        rules: &RoutingRuleSet,
        subscriptions: &[Subscription],
    ) -> Self {
        Self {
            paths: paths.clone(),
            backend_type: settings.backend.backend_type,
            enabled: settings.auto_update_geodata && settings.backend.binary_path.is_some(),
            interval_secs: settings.geodata_update_interval_secs.max(60),
            singbox_rule_set_tags: singbox_rule_set_tags(rules, subscriptions),
        }
    }
}

/// Derives the full sing-box rule-set tags (`geoip-ru`, `geosite-google`, ...)
/// referenced by the enabled global routing rules plus every subscription's
/// active imported profile, for fetching and indexing only the geodata
/// actually in use. A profile's rules are otherwise invisible to this scan -
/// without it, sing-box hits an uncached category at connect time and does a
/// live, blocking, FATAL-on-failure fetch instead.
pub(crate) fn singbox_rule_set_tags(
    rules: &RoutingRuleSet,
    subscriptions: &[Subscription],
) -> Vec<String> {
    let mut tags: Vec<String> = rules
        .rules()
        .iter()
        .filter(|r| r.enabled)
        .filter_map(rule_set_tag)
        .collect();

    for sub in subscriptions {
        if !sub.use_imported_profile {
            continue;
        }
        let Some(profile) = &sub.imported_profile else {
            continue;
        };
        tags.extend(
            profile
                .rules
                .iter()
                .filter(|r| r.enabled)
                .filter_map(rule_set_tag),
        );
    }

    tags.sort();
    tags.dedup();
    tags
}

fn rule_set_tag(rule: &RoutingRule) -> Option<String> {
    match &rule.match_condition {
        // No downloadable rule-set exists for this pseudo-category (see
        // singbox.rs's ip_is_private handling) - nothing to prefetch.
        RuleMatch::GeoIp { country_code } if country_code.eq_ignore_ascii_case("private") => None,
        RuleMatch::GeoIp { country_code } => Some(format!("geoip-{}", country_code.to_lowercase())),
        RuleMatch::GeoSite { category } => Some(format!("geosite-{}", category.to_lowercase())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use v2ray_rs_core::models::{ImportedProfile, ProxyNode, RuleAction, SubscriptionNode};
    use v2ray_rs_core::models::{TransportSettings, VlessConfig};

    fn geo_rule(condition: RuleMatch, enabled: bool) -> RoutingRule {
        RoutingRule {
            id: uuid::Uuid::new_v4(),
            match_condition: condition,
            action: RuleAction::Proxy,
            enabled,
            group: None,
        }
    }

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

    #[test]
    fn geoip_private_is_never_collected_as_a_downloadable_tag() {
        let rules = RoutingRuleSet::default();
        let mut rules = rules;
        rules
            .add_at(
                0,
                geo_rule(
                    RuleMatch::GeoIp {
                        country_code: "PRIVATE".into(),
                    },
                    true,
                ),
            )
            .unwrap();

        let tags = singbox_rule_set_tags(&rules, &[]);

        assert!(tags.is_empty(), "private has no .srs to fetch: {tags:?}");
    }

    #[test]
    fn active_imported_profile_categories_are_included() {
        let mut sub = Subscription::new_from_url("Provider", "https://example.com/sub");
        sub.nodes = vec![SubscriptionNode::new(vless_node())];
        sub.use_imported_profile = true;
        sub.imported_profile = Some(ImportedProfile {
            rules: vec![geo_rule(
                RuleMatch::GeoSite {
                    category: "netflix".into(),
                },
                true,
            )],
            dns: None,
            skipped: vec![],
            imported_at: chrono::Utc::now(),
        });

        let tags = singbox_rule_set_tags(&RoutingRuleSet::default(), &[sub]);

        assert_eq!(tags, vec!["geosite-netflix".to_string()]);
    }

    #[test]
    fn inactive_imported_profile_is_not_prefetched() {
        let mut sub = Subscription::new_from_url("Provider", "https://example.com/sub");
        sub.nodes = vec![SubscriptionNode::new(vless_node())];
        sub.use_imported_profile = false;
        sub.imported_profile = Some(ImportedProfile {
            rules: vec![geo_rule(
                RuleMatch::GeoSite {
                    category: "netflix".into(),
                },
                true,
            )],
            dns: None,
            skipped: vec![],
            imported_at: chrono::Utc::now(),
        });

        let tags = singbox_rule_set_tags(&RoutingRuleSet::default(), &[sub]);

        assert!(tags.is_empty());
    }
}

#[derive(Clone)]
pub struct GeodataRefreshService {
    config_tx: tokio::sync::watch::Sender<GeodataRefreshConfig>,
}

impl GeodataRefreshService {
    pub fn spawn(initial: GeodataRefreshConfig) -> Self {
        let (config_tx, config_rx) = tokio::sync::watch::channel(initial);
        tokio::spawn(async move {
            run_loop(config_rx).await;
        });
        Self { config_tx }
    }

    pub fn update(&self, config: GeodataRefreshConfig) {
        let _ = self.config_tx.send(config);
    }
}

async fn run_loop(mut config_rx: tokio::sync::watch::Receiver<GeodataRefreshConfig>) {
    loop {
        let config = config_rx.borrow().clone();
        if !config.enabled {
            if config_rx.changed().await.is_err() {
                break;
            }
            continue;
        }

        let refresh_config = config.clone();
        let result = tokio::task::spawn_blocking(move || refresh_once(refresh_config)).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => log::warn!("geodata refresh failed: {err}"),
            Err(err) => log::warn!("geodata refresh task failed: {err}"),
        }

        let sleep = tokio::time::sleep(Duration::from_secs(config.interval_secs));
        tokio::pin!(sleep);

        tokio::select! {
            _ = &mut sleep => {}
            changed = config_rx.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
    }
}

fn refresh_once(config: GeodataRefreshConfig) -> Result<(), String> {
    #[cfg(feature = "geodata-fetch")]
    {
        use v2ray_rs_core::geodata::{
            GeodataManager, check_and_download, download_singbox_rule_sets,
        };
        use v2ray_rs_core::geodata_index::GeodataIndexManager;

        let manager = GeodataManager::new(&config.paths);
        let index_manager = GeodataIndexManager::new(&config.paths);

        if config.backend_type == BackendType::SingBox {
            let index_missing = index_manager
                .load_index(BackendType::SingBox)
                .map_err(|err| err.to_string())?
                .is_none();

            let missing: Vec<String> = config
                .singbox_rule_set_tags
                .iter()
                .filter(|tag| !manager.has_rule_set(tag))
                .cloned()
                .collect();

            if !missing.is_empty() {
                download_singbox_rule_sets(&manager, &missing).map_err(|err| err.to_string())?;
            }

            if !missing.is_empty() || index_missing {
                manager
                    .reindex(BackendType::SingBox)
                    .map_err(|err| err.to_string())?;
            }
        } else {
            let interval = Duration::from_secs(config.interval_secs);
            let changed = check_and_download(&manager, interval).map_err(|err| err.to_string())?;

            let index_missing = index_manager
                .load_index(config.backend_type)
                .map_err(|err| err.to_string())?
                .is_none();

            if changed.is_some() || index_missing {
                manager
                    .reindex(config.backend_type)
                    .map_err(|err| err.to_string())?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "geodata-fetch"))]
    {
        let _ = config;
        Ok(())
    }
}
