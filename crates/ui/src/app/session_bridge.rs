use super::*;
pub(super) fn origin_field(origin: ConnectOrigin, direct: bool) -> &'static str {
    match origin {
        ConnectOrigin::User if direct => "node",
        ConnectOrigin::User => "user",
        ConnectOrigin::AutoReconnect => "auto-reconnect",
        ConnectOrigin::Restart => "restart",
    }
}

pub(super) fn strategy_field(strategy: AutoResolveStrategy) -> &'static str {
    match strategy {
        AutoResolveStrategy::ListOrder => "list-order",
        AutoResolveStrategy::LowestLatency => "lowest-latency",
        AutoResolveStrategy::Random => "random",
        AutoResolveStrategy::LastSuccessful => "last-successful",
    }
}

pub(super) fn connect_line(
    origin: ConnectOrigin,
    direct: bool,
    strategy: AutoResolveStrategy,
    candidates: usize,
    profile_override: bool,
) -> String {
    format!(
        "connect origin={} strategy={} candidates={candidates} profile_override={profile_override}",
        origin_field(origin, direct),
        strategy_field(strategy),
    )
}

pub(super) fn auto_reconnect_line(attempt: u32, max: u32) -> String {
    format!(
        "auto-reconnect scheduled attempt={attempt}/{max} delay={}s",
        AUTO_RECONNECT_DELAY.as_secs()
    )
}

pub(super) fn auto_reconnect_exhausted_line() -> String {
    format!("auto-reconnect exhausted max={MAX_AUTO_RECONNECTS}")
}

pub(super) fn health_failover_line(node: &str, count: u32, max: u32) -> String {
    format!("health failover node={node} count={count}/{max}")
}

/// Returns `(shows_disconnect, sensitive)`. `Starting` offers Disconnect so a
/// slow connect can be cancelled; `Stopping` has nothing left to cancel.
pub(super) fn connect_toggle(state: &ProcessState) -> (bool, bool) {
    match state {
        ProcessState::Stopped | ProcessState::Error(_) => (false, true),
        ProcessState::Starting | ProcessState::Running => (true, true),
        ProcessState::Stopping => (true, false),
    }
}

pub(super) fn auto_reconnect_allowed(pending_exit: bool, attempts: u32) -> bool {
    !pending_exit && attempts < MAX_AUTO_RECONNECTS
}

/// While a failed cleanup holds the old manager, neither manual connects nor
/// automatic failover may start: the host may still hold that session's
/// rules, and the retry action owns it.
pub(super) fn cleanup_blocks_new_session(cleanup_pending: bool) -> bool {
    cleanup_pending
}

/// Retry needs the pending flag, the retained session handle, and a failure
/// a retry can actually reach; an unverified engine exit has none.
pub(super) fn cleanup_retry_available(
    cleanup_pending: bool,
    has_handle: bool,
    recovery_required: bool,
) -> bool {
    cleanup_pending && has_handle && !recovery_required
}

/// While a failed cleanup owns the UI, its error text is the banner's
/// message: a matching `apply_state` toast would repeat it. Any other error
/// still toasts.
pub(super) fn cleanup_error_toast_suppressed(
    cleanup_pending: bool,
    cleanup_message: Option<&str>,
    error: &str,
) -> bool {
    cleanup_pending && cleanup_message == Some(error)
}

/// Only user- and restart-initiated connects invalidate a pending
/// auto-reconnect; the timer's own connect must leave the attempt budget
/// counting toward MAX_AUTO_RECONNECTS.
pub(super) fn cancels_auto_reconnect(origin: ConnectOrigin) -> bool {
    origin != ConnectOrigin::AutoReconnect
}

/// The message a terminal state reconnects with: an anchored session goes
/// back to its own node; a plan without a target replans.
pub(super) fn reconnect_msg(target: Option<SessionTarget>, origin: ConnectOrigin) -> AppMsg {
    match target {
        Some(session) => AppMsg::ConnectToNode(session.node, origin),
        None => AppMsg::Connect(origin),
    }
}

/// A lost node surfaces to the user only when the user picked it; restarts
/// and auto-reconnects fall back to the configured strategy instead.
pub(super) fn falls_back_to_planner(origin: ConnectOrigin) -> bool {
    origin != ConnectOrigin::User
}

/// The scheduled timer fires only for the generation it was armed with and
/// only while no connection is up.
pub(super) fn auto_reconnect_fires(message: u32, current: u32, has_handle: bool) -> bool {
    message == current && !has_handle
}

/// Log and state messages carry the generation of the connection that
/// produced them; only the app's current one is live.
pub(super) fn is_current_generation(message: u64, current: u64) -> bool {
    message == current
}

/// One degraded-DPI notice per generation, only while that generation is the
/// live connection and its session is actually `Running`: a superseded
/// session's report, a repeated report, and a queued notice arriving during
/// teardown or after Disconnect all stay silent — the proxy no longer stays
/// up for a stopped session.
pub(super) fn engine_degraded_notice_fires(
    seen: Option<u64>,
    message: u64,
    current: u64,
    state: &ProcessState,
) -> bool {
    is_current_generation(message, current)
        && seen != Some(message)
        && matches!(state, ProcessState::Running)
}

/// Keeps the addresses the current connection pinned, replacing whatever an
/// earlier session pinned for the same hostnames.
pub(super) fn apply_node_pins(
    store: &mut Vec<HostOverride>,
    message_generation: u64,
    current_generation: u64,
    pins: Vec<HostOverride>,
) {
    if !is_current_generation(message_generation, current_generation) {
        return;
    }
    store.retain(|kept| !pins.iter().any(|pin| pin.domain == kept.domain));
    store.extend(pins);
}

pub(super) fn tun_ipv6_unavailable(
    tun: &TunConfig,
    backend: BackendType,
    host_has_ipv6: bool,
) -> bool {
    tun.enabled
        && matches!(backend, BackendType::SingBox | BackendType::Xray)
        && tun.address_v6.is_some()
        && !host_has_ipv6
}

/// Only sessions picked by the configured strategy fail over for health, only
/// when another node is left to try, and only within a budget that a
/// successful probe or a user action refills.
pub(super) fn health_failover_allowed(
    enabled: bool,
    session_target: Option<SessionTarget>,
    failover: &HealthFailover,
) -> bool {
    enabled
        && session_target.is_none()
        && failover.candidates > 1
        && failover.count < MAX_AUTO_RECONNECTS
}

pub(super) fn exclude_candidate(
    candidates: Vec<ConnectionCandidate>,
    excluded: Option<ConnectionNodeRef>,
) -> Vec<ConnectionCandidate> {
    let Some(excluded) = excluded else {
        return candidates;
    };
    candidates
        .into_iter()
        .filter(|candidate| candidate.node_ref != excluded)
        .collect()
}

/// A direct connect starts unestablished; only the reconnect timer re-enters
/// a session that had already been established.
pub(super) fn direct_session(node: ConnectionNodeRef, origin: ConnectOrigin) -> SessionTarget {
    SessionTarget {
        node,
        established: origin == ConnectOrigin::AutoReconnect,
    }
}

/// `Running` anchors the session: its target becomes established.
pub(super) fn session_target_after_running(target: Option<SessionTarget>) -> Option<SessionTarget> {
    target.map(|mut session| {
        session.established = true;
        session
    })
}

/// A stop keeps the target only while a restart will follow; a session the
/// user ended drops it.
pub(super) fn session_target_after_stop(
    target: Option<SessionTarget>,
    reconnect_pending: bool,
) -> Option<SessionTarget> {
    if reconnect_pending { target } else { None }
}

/// An unestablished direct target must surface its failure instead of
/// retrying; established and planned sessions follow the normal release
/// decision.
pub(super) fn retry_after_error(
    target: Option<SessionTarget>,
    app_state_stopping: bool,
    reconnects_left: u32,
) -> bool {
    match target {
        Some(session) if !session.established => false,
        _ => !release_on_error(app_state_stopping, reconnects_left),
    }
}

/// Pure helper for terminal-state direct-connect bookkeeping: a terminal
/// `Stopped` or `Error` replays a pending direct target exactly once, so the
/// revalidation path decides the next state.
pub(super) fn consume_terminal_direct_state(
    state: &ProcessState,
    pending_direct_target: &mut Option<ConnectionNodeRef>,
) -> Option<ConnectionNodeRef> {
    match state {
        ProcessState::Stopped | ProcessState::Error(_) => pending_direct_target.take(),
        _ => None,
    }
}

/// A settings flush from the preferences dialog carries a copy taken when the
/// dialog opened; a connection may have refreshed `last_success` since, so the
/// stale copy must not overwrite the app's current record.
pub(super) fn keep_last_success(mut incoming: AppSettings, current: &AppSettings) -> AppSettings {
    incoming.last_success = current.last_success.clone();
    incoming
}

/// Only a backend that became ready counts as a success; `Starting` and
/// `Stopping` relays carry metadata for display alone.
pub(super) fn records_last_success(
    state: &ProcessState,
    connection: Option<&ConnectionMetadata>,
) -> bool {
    matches!(state, ProcessState::Running) && connection.is_some()
}

/// A session that reached `Running` records which node last served traffic,
/// seeding the `LastSuccessful` auto-resolve strategy.
pub(super) fn last_success_settings(
    current: &AppSettings,
    meta: &ConnectionMetadata,
) -> AppSettings {
    let mut settings = current.clone();
    settings.last_success = Some(LastSuccessMetadata {
        node_ref: meta.node_ref,
        connected_at: meta.connected_since,
    });
    settings
}

pub(super) fn should_announce(prev: Option<&Health>, next: &Health) -> bool {
    matches!(next, Health::Unhealthy(_)) && !matches!(prev, Some(Health::Unhealthy(_)))
}

pub(super) fn active_nodes_available(
    subscriptions: &[Subscription],
    manual_nodes: &[ManualNode],
) -> bool {
    subscriptions
        .iter()
        .any(|subscription| subscription.has_enabled_nodes())
        || manual_nodes.iter().any(|node| node.enabled)
}
