use super::*;
/// Coalesces config-regeneration requests so rapid changes produce one write
/// of the newest state instead of stacking blocking work on the main loop.
#[derive(Debug, Default)]
pub(super) struct RegenScheduler {
    in_flight: bool,
    pending: bool,
}

pub(super) fn runtime_process_active(state: &ProcessState) -> bool {
    matches!(state, ProcessState::Starting | ProcessState::Running)
}

pub(super) fn restart_banner_visible_for_state(
    restart_required: bool,
    state: &ProcessState,
) -> bool {
    restart_required && runtime_process_active(state)
}

pub(super) fn reconnect_after_stop(state: &ProcessState, reconnect_pending: bool) -> bool {
    reconnect_pending && matches!(state, ProcessState::Stopped | ProcessState::Error(_))
}

/// v2ray and xray read `geoip:`/`geosite:` references from local .dat files and
/// refuse to start without them; sing-box rule-sets are fetched per tag elsewhere.
pub(super) fn missing_geodata(
    backend: BackendType,
    rules: &[RoutingRule],
    subscriptions: &[Subscription],
    dns: &DnsConfig,
    geoip_exists: bool,
    geosite_exists: bool,
) -> bool {
    if backend == BackendType::SingBox || (geoip_exists && geosite_exists) {
        return false;
    }
    let geo_rule = |r: &RoutingRule| {
        r.enabled
            && matches!(
                r.match_condition,
                RuleMatch::GeoIp { .. } | RuleMatch::GeoSite { .. }
            )
    };
    let geo_dns = |d: &DnsConfig| {
        d.enabled
            && d.use_custom_rules
            && d.rules
                .iter()
                .any(|r| matches!(r.match_condition, DnsRuleMatch::GeoSite { .. }))
    };
    rules.iter().any(geo_rule)
        || geo_dns(dns)
        || subscriptions
            .iter()
            .filter(|s| s.use_imported_profile)
            .filter_map(|s| s.imported_profile.as_ref())
            .any(|p| p.rules.iter().any(geo_rule) || p.dns.as_ref().is_some_and(geo_dns))
}

/// TUN follows the session that is running: the launched snapshot decides,
/// not the current settings, which may already have been edited mid-session.
pub(super) fn tun_active_for(
    state: &ProcessState,
    snapshot: Option<&RuntimeConfigSnapshot>,
) -> bool {
    matches!(state, ProcessState::Running)
        && snapshot.is_some_and(|s| {
            s.tun.enabled && s.backend_type != v2ray_rs_core::models::BackendType::V2ray
        })
}

/// Routing-rules load guard for config regeneration: the caller reports this
/// message and skips the config write, so a corrupt rules file can never
/// produce a regenerated config from partial rules.
pub(super) fn regeneration_rules(store: &WorkspaceStore) -> Result<RoutingRuleSet, String> {
    store
        .load_routing_rules()
        .map_err(|err| format!("Failed to load routing rules: {err}"))
}

impl RegenScheduler {
    /// Records a request. Returns true when the caller must start a new job.
    pub(super) fn request(&mut self) -> bool {
        if self.in_flight {
            self.pending = true;
            false
        } else {
            self.in_flight = true;
            true
        }
    }

    /// Closes the running job. Returns true when a coalesced request must rerun.
    pub(super) fn completed(&mut self) -> bool {
        self.in_flight = false;
        std::mem::replace(&mut self.pending, false)
    }
}
