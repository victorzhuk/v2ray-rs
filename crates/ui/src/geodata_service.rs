use crate::failure_streak::FailureStreak;
use std::time::Duration;

use v2ray_rs_core::geodata::canonical_rule_set_tag;
use v2ray_rs_core::models::{
    AppSettings, BackendType, DnsConfig, DnsRuleMatch, RoutingRule, RoutingRuleSet, RuleMatch,
    Subscription,
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
            singbox_rule_set_tags: singbox_rule_set_tags(rules, subscriptions, settings),
        }
    }
}

/// Derives the full sing-box rule-set tags (`geoip-ru`, `geosite-google`, ...)
/// referenced by the enabled global routing rules, the active DNS rule set, and
/// every subscription's active imported profile, for fetching and indexing only
/// the geodata actually in use. A profile's rules are otherwise invisible to
/// this scan - without it, sing-box hits an uncached category at connect time
/// and does a live, blocking, FATAL-on-failure fetch instead.
pub(crate) fn singbox_rule_set_tags(
    rules: &RoutingRuleSet,
    subscriptions: &[Subscription],
    settings: &AppSettings,
) -> Vec<String> {
    let mut tags: Vec<String> = rules
        .rules()
        .iter()
        .filter(|r| r.enabled)
        .filter_map(rule_set_tag)
        .collect();

    dns_geosite_tags(&settings.dns, &mut tags);

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
        if let Some(dns) = &profile.dns {
            dns_geosite_tags(dns, &mut tags);
        }
    }

    tags.sort();
    tags.dedup();
    tags
}

fn dns_geosite_tags(dns: &DnsConfig, tags: &mut Vec<String>) {
    if !dns.enabled || !dns.use_custom_rules {
        return;
    }
    for rule in &dns.rules {
        if let DnsRuleMatch::GeoSite { category } = &rule.match_condition
            && let Some(tag) = canonical_rule_set_tag(&format!("geosite-{category}"))
        {
            tags.push(tag);
        }
    }
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
            via_node: None,
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

        let tags = singbox_rule_set_tags(&rules, &[], &AppSettings::default());

        assert!(tags.is_empty(), "private has no .srs to fetch: {tags:?}");
    }

    #[test]
    fn active_custom_dns_geosite_categories_are_prefetched() {
        let mut settings = AppSettings::default();
        settings.dns.enabled = true;
        settings.dns.use_custom_rules = true;
        settings.dns.rules = vec![v2ray_rs_core::models::DnsRule {
            match_condition: v2ray_rs_core::models::DnsRuleMatch::GeoSite {
                category: "OpenAI".into(),
            },
            server_tag: "remote".into(),
        }];

        let tags = singbox_rule_set_tags(&RoutingRuleSet::default(), &[], &settings);

        assert_eq!(tags, vec!["geosite-openai".to_string()]);
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

        let tags =
            singbox_rule_set_tags(&RoutingRuleSet::default(), &[sub], &AppSettings::default());

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

        let tags =
            singbox_rule_set_tags(&RoutingRuleSet::default(), &[sub], &AppSettings::default());

        assert!(tags.is_empty());
    }

    #[test]
    fn geodata_failure_toasts_once_per_streak() {
        let mut streak = FailureStreak::new();
        assert_eq!(
            refresh_outcome_toast(&mut streak, &Err("boom".into())),
            Some("Geodata refresh failed: boom".to_string())
        );
        assert_eq!(
            refresh_outcome_toast(&mut streak, &Err("boom".into())),
            None
        );
        assert_eq!(
            refresh_outcome_toast(&mut streak, &Err("boom".into())),
            None
        );
        assert_eq!(refresh_outcome_toast(&mut streak, &Ok(())), None);
        assert_eq!(
            refresh_outcome_toast(&mut streak, &Err("boom".into())),
            Some("Geodata refresh failed: boom".to_string())
        );
    }
}

#[derive(Clone)]
pub struct GeodataRefreshService {
    config_tx: tokio::sync::watch::Sender<GeodataRefreshConfig>,
}

impl GeodataRefreshService {
    pub fn spawn(
        initial: GeodataRefreshConfig,
        toasts: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Self {
        let (config_tx, config_rx) = tokio::sync::watch::channel(initial);
        tokio::spawn(async move {
            run_loop(config_rx, toasts).await;
        });
        Self { config_tx }
    }

    pub fn update(&self, config: GeodataRefreshConfig) {
        let _ = self.config_tx.send(config);
    }
}

fn refresh_outcome_toast(
    streak: &mut FailureStreak,
    outcome: &Result<(), String>,
) -> Option<String> {
    match outcome {
        Ok(()) => {
            streak.record_success();
            None
        }
        Err(err) => streak
            .record_failure()
            .then(|| format!("Geodata refresh failed: {err}")),
    }
}

async fn run_loop(
    mut config_rx: tokio::sync::watch::Receiver<GeodataRefreshConfig>,
    toasts: tokio::sync::mpsc::UnboundedSender<String>,
) {
    let mut streak = FailureStreak::new();
    let mut first_pass = true;
    loop {
        let config = config_rx.borrow().clone();
        if !config.enabled {
            if config_rx.changed().await.is_err() {
                break;
            }
            continue;
        }

        let refresh_config = config.clone();
        let force_all = !first_pass;
        first_pass = false;
        let result =
            tokio::task::spawn_blocking(move || refresh_once(refresh_config, force_all)).await;
        let outcome: Result<(), String> = match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => {
                log::warn!("geodata refresh failed: {err}");
                Err(err)
            }
            Err(err) => {
                log::warn!("geodata refresh task failed: {err}");
                Err(err.to_string())
            }
        };
        if let Some(toast) = refresh_outcome_toast(&mut streak, &outcome) {
            let _ = toasts.send(toast);
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

fn refresh_once(config: GeodataRefreshConfig, force_all: bool) -> Result<(), String> {
    #[cfg(feature = "geodata-fetch")]
    {
        use v2ray_rs_core::geodata::{
            GeodataManager, check_and_download, download_singbox_rule_sets, rule_sets_to_fetch,
        };
        use v2ray_rs_core::geodata_index::GeodataIndexManager;

        let manager = GeodataManager::new(&config.paths);
        let index_manager = GeodataIndexManager::new(&config.paths);

        if config.backend_type == BackendType::SingBox {
            let index_missing = index_manager
                .load_index(BackendType::SingBox)
                .map_err(|err| err.to_string())?
                .is_none();

            let fetch = rule_sets_to_fetch(&manager, &config.singbox_rule_set_tags, force_all);

            if !fetch.is_empty() {
                download_singbox_rule_sets(&manager, &fetch, force_all)
                    .map_err(|err| err.to_string())?;
            }

            if !fetch.is_empty() || index_missing {
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

pub(crate) fn update_geodata(paths: &AppPaths, backend: BackendType) -> Result<(), String> {
    #[cfg(feature = "geodata-fetch")]
    {
        use v2ray_rs_core::geodata::{
            GeodataManager, download_geodata, download_singbox_rule_sets, rule_sets_to_fetch,
        };
        use v2ray_rs_core::geodata_index::GeodataIndexManager;
        use v2ray_rs_core::persistence::{
            load_routing_rules, load_settings_or_default, load_subscriptions,
        };

        let geodata_manager = GeodataManager::new(paths);
        let index_manager = GeodataIndexManager::new(paths);

        if backend == BackendType::SingBox {
            let rules = load_routing_rules(paths).unwrap_or_default();
            let subscriptions = load_subscriptions(paths).unwrap_or_default();
            let settings = load_settings_or_default(paths);
            let tags = singbox_rule_set_tags(&rules, &subscriptions, &settings);
            let fetch = rule_sets_to_fetch(&geodata_manager, &tags, true);

            if !fetch.is_empty() {
                download_singbox_rule_sets(&geodata_manager, &fetch, true)
                    .map_err(|e| format!("Download failed: {}", e))?;
            }

            index_manager
                .build_singbox_index(&geodata_manager.rule_sets_dir())
                .map_err(|e| format!("Index build failed: {}", e))?;
        } else {
            download_geodata(&geodata_manager).map_err(|e| format!("Download failed: {}", e))?;

            let geoip_path = geodata_manager.geoip_path();
            let geosite_path = geodata_manager.geosite_path();

            index_manager
                .build_index(backend, &geoip_path, &geosite_path)
                .map_err(|e| format!("Index build failed: {}", e))?;
        }

        Ok(())
    }

    #[cfg(not(feature = "geodata-fetch"))]
    {
        let _ = (paths, backend);
        Err("Geodata download feature not enabled".to_string())
    }
}
