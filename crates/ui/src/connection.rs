use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, broadcast, mpsc};
use tokio::task::JoinHandle;
use v2ray_rs_core::config::ConfigWriter;
use v2ray_rs_core::models::{
    AppSettings, BackendType, ConnectionMetadata, ConnectionNodeRef, DnsHijackMode, HostOverride,
    ManualNode, ProxyNode, RoutingRule, Subscription, resolve_effective_config,
    uses_imported_profile,
};
use v2ray_rs_core::persistence::{AppPaths, TunSession, save_tun_session};
use v2ray_rs_core::resolve::{ConnectionCandidate, resolve_via_nodes};
use v2ray_rs_core::rotating_log::{DEFAULT_MAX_BYTES, RotatingFileWriter};
use v2ray_rs_process::{
    ProcessError, ProcessEvent, ProcessManager, ProcessState, StopReason, TunRuntime,
};

use crate::app::AppMsg;
use crate::health::{DnsFailureWindow, HealthTracker};

/// Serializes everything that mutates backend-process and kernel TUN state.
///
/// `netctl` has no session identity: it deletes devices by interface name and
/// policy rules by fixed priority. A teardown therefore removes whatever
/// currently occupies those names and priorities, not specifically the session
/// that installed them. Since `Disconnect` clears the handle before teardown has
/// run, a prompt reconnect would otherwise set up a new session that the old
/// task's `xray-down` then deletes — leaving a live backend, no tunnel, and a UI
/// reporting Connected. Holding this for a connection's whole lifetime makes the
/// next connect wait for the previous teardown instead of racing it.
pub(super) type TunLifecycle = Arc<Mutex<()>>;

pub(super) struct ConnectionHandle {
    cmd_tx: mpsc::Sender<ConnectionCmd>,
}

pub(super) struct ConnectionRequest {
    pub binary_path: PathBuf,
    pub candidates: Vec<ConnectionCandidate>,
    pub writer: ConfigWriter,
    pub paths: AppPaths,
    pub pid_path: PathBuf,
    pub geodata_dir: PathBuf,
    pub settings: AppSettings,
    pub enabled_rules: Vec<RoutingRule>,
    pub subscriptions: Vec<Subscription>,
    pub manual_nodes: Vec<ManualNode>,
    pub lifecycle: TunLifecycle,
    /// Identifies this connection attempt. A task that lost the handle to a
    /// newer connect keeps running until its teardown finishes and still reports
    /// its own terminal state; the app drops those so a stale `Stopped` cannot
    /// clear the live connection's handle.
    pub generation: u64,
    pub host_has_ipv6: bool,
    pub health_timing: HealthTiming,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct HealthTiming {
    pub initial_delay: Duration,
    pub interval: Duration,
    /// Re-evaluates the DNS failure window without a new log line, so the
    /// failing mark clears once a burst has stopped.
    pub dns_recheck: Duration,
}

impl Default for HealthTiming {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(10),
            interval: Duration::from_secs(30),
            dns_recheck: Duration::from_secs(10),
        }
    }
}

enum ConnectionCmd {
    Stop(StopReason),
}

impl ConnectionHandle {
    pub(super) fn stop(&self, reason: StopReason) {
        let _ = self.cmd_tx.try_send(ConnectionCmd::Stop(reason));
    }
}

pub(super) fn spawn(request: ConnectionRequest, sender: relm4::Sender<AppMsg>) -> ConnectionHandle {
    spawn_with(request, sender, |mgr| mgr)
}

/// `spawn` with a hook over every candidate's manager, applied after the
/// production builder chain.
fn spawn_with(
    request: ConnectionRequest,
    sender: relm4::Sender<AppMsg>,
    configure: impl Fn(ProcessManager) -> ProcessManager + Send + 'static,
) -> ConnectionHandle {
    let ConnectionRequest {
        binary_path,
        candidates,
        writer,
        paths,
        pid_path,
        geodata_dir,
        settings,
        enabled_rules,
        subscriptions,
        manual_nodes,
        lifecycle,
        generation,
        host_has_ipv6,
        health_timing,
    } = request;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<ConnectionCmd>(4);

    tokio::spawn(async move {
        // Held until this task returns, by every exit path including the early
        // `return`s below, so the next connect cannot start setting up while
        // this one is still tearing down.
        let _lifecycle = lifecycle.lock().await;
        let report = |state: ProcessState, connection: Option<ConnectionMetadata>| {
            sender.emit(AppMsg::ProcessStateConnection(
                generation, state, connection,
            ));
        };

        // A Stop that arrived while we were queued behind the previous
        // connection's teardown must not be answered by starting anyway.
        if matches!(cmd_rx.try_recv(), Ok(ConnectionCmd::Stop(_))) {
            report(ProcessState::Stopped, None);
            return;
        }

        // Reap any orphaned backend from a previous run before spawning ours.
        // Done here, off the GTK thread, so the Connect click never blocks the
        // UI while an orphan is signalled and waited on.
        {
            let orphan_pid = pid_path.clone();
            let _ = tokio::task::spawn_blocking(move || {
                v2ray_rs_process::PidFile::new(orphan_pid).check_and_kill_orphaned()
            })
            .await;
        }

        let mut failures = Vec::new();
        // A failed candidate keeps its routing state while the next one starts,
        // so traffic does not leak between attempts; only a Stop releases it.
        let mut parked: Option<ProcessManager> = None;
        // One writer spans every candidate attempt so failovers append to the
        // same backend.log; an open failure only costs the file diagnostics.
        let backend_log =
            match RotatingFileWriter::open(paths.logs_dir().join("backend.log"), DEFAULT_MAX_BYTES)
            {
                Ok(writer) => Some(Arc::new(writer)),
                Err(err) => {
                    log::warn!("open backend log: {err}");
                    None
                }
            };

        // Decided once per connection so the notice cannot repeat per candidate.
        let drop_strict_route = settings.tun.enabled
            && settings.tun.strict_route
            && !singbox_strict_route_allowed(settings.backend.backend_type, host_has_ipv6);
        if drop_strict_route {
            if let Some(log) = &backend_log {
                log.append_line("notice", STRICT_ROUTE_NOTICE);
            }
            sender.emit(AppMsg::ProcessLogLine(
                generation,
                STRICT_ROUTE_NOTICE.into(),
            ));
        }

        // Same reasoning as the strict-route notice: outside the candidate loop
        // so a failover cannot repeat it.
        if let Some(warning) = v2ray_tun_warning(&settings) {
            if let Some(log) = &backend_log {
                log.append_line("warning", &warning);
            }
            sender.emit(AppMsg::ProcessLogLine(generation, warning));
        }

        let total = candidates.len();
        'candidates: for (index, candidate) in candidates.into_iter().enumerate() {
            let position = index + 1;
            if let Ok(ConnectionCmd::Stop(reason)) = cmd_rx.try_recv() {
                if let Some(mut failed) = parked.take() {
                    failed.shutdown_with(reason).await;
                }
                report(ProcessState::Stopped, None);
                return;
            }
            let candidate_label = candidate
                .node
                .remark()
                .unwrap_or(candidate.node.address())
                .to_string();
            let candidate_address = candidate.node.address().to_string();
            log::info!("candidate start {position}/{total} label={candidate_label}");
            let candidate_port = candidate.node.port();
            let (mut effective_rules, mut effective_settings) = resolve_effective_config(
                &candidate.node_ref,
                &subscriptions,
                &enabled_rules,
                &settings,
            );
            // Rules pinned to another node need that node in the outbound list.
            // The connected node stays first so it remains the default target.
            let mut nodes = vec![candidate.node.clone()];
            nodes.extend(resolve_via_nodes(
                &mut effective_rules,
                &subscriptions,
                &manual_nodes,
            ));
            // Must happen before the tunnel exists: once its rules are up, this
            // very lookup would be captured by the tunnel it is preparing.
            pin_node_addresses(&mut effective_settings, &nodes).await;
            if drop_strict_route {
                effective_settings.tun.strict_route = false;
            }
            let pinned = hosts_cover_nodes(&effective_settings, &nodes);
            let config_path =
                match writer.write_config(&nodes, &effective_rules, &effective_settings) {
                    Ok(path) => path,
                    Err(e) => {
                        record_failure(
                            &mut failures,
                            CandidateFailure::new(
                                &candidate_label,
                                &format!("config generation failed: {e}"),
                                &candidate_address,
                                candidate_port,
                            ),
                            position,
                            total,
                        );
                        if repeats_previous(&failures) {
                            break 'candidates;
                        }
                        continue;
                    }
                };

            let meta = ConnectionMetadata {
                node_ref: candidate.node_ref,
                source: candidate.source_name,
                source_id: match &candidate.node_ref {
                    ConnectionNodeRef::Subscription {
                        subscription_id, ..
                    } => subscription_id.to_string(),
                    ConnectionNodeRef::Manual { node_id } => node_id.to_string(),
                },
                node_name: candidate_label.clone(),
                node_address: candidate.node.address().to_string(),
                node_port: candidate.node.port(),
                backend: settings.backend.backend_type,
                strategy: settings.auto_resolve_strategy,
                latency_ms: candidate.latency_ms,
                connected_since: chrono::Utc::now(),
            };

            let tun = build_tun_runtime(&effective_settings, pinned);
            let session_fields = format!(
                "hijack={} capture_dns={} strict={} nodes_pinned={pinned} profile={}",
                tun.as_ref()
                    .map_or("off", |_| hijack_field(effective_settings.tun.dns_hijack)),
                tun.as_ref().is_some_and(|rt| rt.capture_dns),
                tun.as_ref().is_some_and(|rt| rt.strict),
                if uses_imported_profile(&candidate.node_ref, &subscriptions) {
                    "imported"
                } else {
                    "app"
                },
            );
            // Written before the backend or the route helper touches the kernel,
            // so a crash mid-start still leaves the next launch a recovery pass.
            if let Some(rt) = &tun
                && let Err(err) = save_tun_session(&paths, &tun_session_for(rt))
            {
                log::warn!("save tun session marker: {err}");
            }
            let mut mgr = configure(
                ProcessManager::new(
                    binary_path.clone(),
                    config_path,
                    pid_path.clone(),
                    Some(geodata_dir.clone()),
                )
                .with_tun(tun)
                .with_backend(settings.backend.backend_type)
                .with_log_file(backend_log.clone())
                .with_session_fields(session_fields)
                .with_ready_probe(effective_settings.local_endpoint(effective_settings.socks_port)),
            );

            let started = tokio::select! {
                biased;
                Some(ConnectionCmd::Stop(reason)) = cmd_rx.recv() => {
                    mgr.shutdown_with(reason).await;
                    if let Some(mut failed) = parked.take() {
                        failed.shutdown_with(reason).await;
                    }
                    report(ProcessState::Stopped, None);
                    return;
                }
                started = mgr.start_with_connection(Some(meta.clone())) => started,
            };

            match started {
                Ok(()) => {
                    // A Disconnect clicked while the start was in flight sits
                    // queued until here; honor it instead of flashing the UI
                    // back to Connected with a dead handle.
                    if let Ok(ConnectionCmd::Stop(reason)) = cmd_rx.try_recv() {
                        mgr.shutdown_with(reason).await;
                        report(ProcessState::Stopped, None);
                        return;
                    }
                    report(ProcessState::Running, Some(meta.clone()));
                }
                Err(e) => {
                    if e.is_host_level() {
                        // A host-level failure blocks every candidate, so
                        // failover is over: tear down what is parked and
                        // report the host error itself, not a summary.
                        mgr.shutdown_with(StopReason::StartFailed).await;
                        if let Some(mut failed) = parked.take() {
                            failed.shutdown_with(StopReason::StartFailed).await;
                        }
                        if grant_fixable(&e) {
                            sender.emit(AppMsg::TunGrantRequired(generation));
                        }
                        report(ProcessState::Error(e.to_string()), None);
                        return;
                    }
                    record_failure(
                        &mut failures,
                        CandidateFailure::new(
                            &candidate_label,
                            &e.to_string(),
                            &candidate_address,
                            candidate_port,
                        ),
                        position,
                        total,
                    );
                    parked = Some(mgr);
                    if repeats_previous(&failures) {
                        break 'candidates;
                    }
                    continue;
                }
            }

            let state_sender = sender.clone();
            let mut state_rx = mgr.subscribe();
            let state_forwarder = tokio::spawn(async move {
                loop {
                    match state_rx.recv().await {
                        Ok(ProcessEvent::StateChanged { to, connection, .. }) => {
                            // Terminal states stay with the supervising loop,
                            // which may fail over to the next candidate; only
                            // it decides what the app finally sees.
                            if relays(&to) {
                                state_sender.emit(AppMsg::ProcessStateConnection(
                                    generation, to, connection,
                                ));
                            }
                        }
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            let log_sender = sender.clone();
            let backend = settings.backend.backend_type;
            let mut log_rx = mgr.subscribe_logs();
            let log_forwarder = tokio::spawn(async move {
                // Only xray's log lines are known to mark DNS failures.
                let mut dns = (backend == BackendType::Xray).then(DnsFailureWindow::default);
                let mut dns_failing = false;
                let mut recheck = tokio::time::interval(health_timing.dns_recheck);
                loop {
                    tokio::select! {
                        event = log_rx.recv() => match event {
                            Ok(ProcessEvent::LogLine(line)) => {
                                if let Some(window) = &mut dns {
                                    window.observe(&line.content, Instant::now());
                                }
                                log_sender.emit(AppMsg::ProcessLogLine(generation, line.content));
                            }
                            Ok(_) => continue,
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        },
                        _ = recheck.tick(), if dns.is_some() => {}
                    }
                    if let Some(window) = &dns
                        && let Some(failing) = dns_flip(window, &mut dns_failing, Instant::now())
                    {
                        log_sender.emit(AppMsg::DnsHealth(generation, failing));
                    }
                }
            });
            let mut forwarders = vec![state_forwarder, log_forwarder];
            if effective_settings.health_check.enabled {
                forwarders.push(spawn_health_monitor(
                    HealthProbe {
                        proxy: effective_settings.local_endpoint(effective_settings.http_port),
                        url: effective_settings.real_delay.test_url.clone(),
                        timeout: Duration::from_millis(u64::from(
                            effective_settings.real_delay.timeout_ms,
                        )),
                    },
                    health_timing,
                    mgr.subscribe(),
                    sender.clone(),
                    generation,
                ));
            }

            loop {
                tokio::select! {
                    Some(ConnectionCmd::Stop(reason)) = cmd_rx.recv() => {
                        mgr.shutdown_with(reason).await;
                        halt(forwarders).await;
                        report(ProcessState::Stopped, None);
                        return;
                    }
                    _ = mgr.wait_and_handle_exit() => {
                        // The manager restarts in place on an unexpected exit; if
                        // it came back Running keep supervising. A crash give-up
                        // (Error) falls through to the next candidate; anything
                        // else is a requested stop.
                        match mgr.state() {
                            ProcessState::Running => {}
                            ProcessState::Error(msg) => {
                                record_failure(
                                    &mut failures,
                                    CandidateFailure::new(
                                        &candidate_label,
                                        &msg,
                                        &candidate_address,
                                        candidate_port,
                                    ),
                                    position,
                                    total,
                                );
                                break;
                            }
                            _ => {
                                halt(forwarders).await;
                                report(ProcessState::Stopped, None);
                                return;
                            }
                        }
                    }
                }
            }
            halt(forwarders).await;
            parked = Some(mgr);
            if repeats_previous(&failures) {
                break 'candidates;
            }
        }

        report(ProcessState::Error(terminal_failure(&failures)), None);
    });

    ConnectionHandle { cmd_tx }
}

const STRICT_ROUTE_NOTICE: &str = "notice: kernel IPv6 is disabled; sing-box strict_route turned off for this session (IPv4 routing unchanged)";

const PIN_LOOKUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Pins every hostname-addressed node to concrete IPs in `dns.hosts`. Every
/// family is carried; the generator keeps what its backend can use.
async fn pin_node_addresses(settings: &mut AppSettings, nodes: &[ProxyNode]) {
    for node in nodes {
        let host = node.address();
        if host.parse::<std::net::IpAddr>().is_ok() {
            continue;
        }
        if settings.dns.hosts.iter().any(|h| h.domain == host) {
            continue;
        }

        // A reconnect can start while the previous tunnel's rules are still up,
        // which is exactly when this lookup gets captured and stalls. Bounded so
        // that costs a few seconds and a disabled capture, not the connect.
        let lookup = tokio::time::timeout(
            PIN_LOOKUP_TIMEOUT,
            tokio::net::lookup_host((host, node.port())),
        )
        .await;

        match lookup {
            Err(_) => log::warn!("cannot pin {host}: lookup timed out"),
            Ok(Err(err)) => log::warn!("cannot pin {host}: {err}"),
            Ok(Ok(addrs)) => {
                for addr in addrs {
                    settings.dns.hosts.push(HostOverride {
                        domain: host.to_string(),
                        ip: addr.ip().to_string(),
                    });
                }
            }
        }
    }
}

/// Whether the settings the config is generated from resolve every node
/// hostname on their own. Capturing port 53 while the backend still needs the
/// OS resolver to find its own server would send that lookup into the tunnel it
/// is trying to build. An override xray answers with an empty set — the wrong
/// family for its query strategy — does not count.
fn hosts_cover_nodes(settings: &AppSettings, nodes: &[ProxyNode]) -> bool {
    nodes.iter().all(|node| {
        let host = node.address();
        host.parse::<std::net::IpAddr>().is_ok()
            || settings
                .dns
                .hosts
                .iter()
                .any(|h| h.domain == host && h.matches_strategy(settings.dns.strategy))
    })
}

fn hijack_field(mode: DnsHijackMode) -> &'static str {
    match mode {
        DnsHijackMode::Hijack => "hijack",
        DnsHijackMode::Native => "native",
        DnsHijackMode::Disabled => "disabled",
    }
}

/// Builds the TUN runtime from settings, or `None` when TUN is off or the
/// backend is v2ray (which has no native TUN inbound).
fn build_tun_runtime(settings: &AppSettings, nodes_pinned: bool) -> Option<TunRuntime> {
    if !settings.tun.enabled {
        return None;
    }
    let backend = settings.backend.backend_type;
    if backend == BackendType::V2ray {
        return None;
    }
    let bypass_uid = nix::unistd::User::from_name(v2ray_rs_process::BYPASS_USER)
        .ok()
        .flatten()
        .map(|u| u.uid.as_raw());
    Some(TunRuntime {
        backend,
        iface: settings.tun.interface_name.clone(),
        addr_v4: settings.tun.address_v4.clone(),
        addr_v6: settings.tun.address_v6.clone(),
        helper_path: v2ray_rs_process::helper_path(),
        bypass_uid,
        // sing-box captures DNS itself via auto_route; only the xray path needs
        // the policy rule, and only when the config actually hijacks port 53.
        // `nodes_pinned` is the safety interlock: a node the generated config
        // cannot resolve on its own still needs the OS resolver to be found,
        // and capturing port 53 would swallow that lookup.
        capture_dns: backend == BackendType::Xray
            && settings.tun.dns_hijack == DnsHijackMode::Hijack
            && nodes_pinned,
        strict: settings.tun.strict_route,
    })
}

/// The warning to surface when TUN is on but the backend is v2ray, which has no
/// native TUN inbound: nothing is tunnelled, only the proxy endpoints work.
pub(super) fn v2ray_tun_warning(settings: &AppSettings) -> Option<String> {
    if !settings.tun.enabled || settings.backend.backend_type != BackendType::V2ray {
        return None;
    }
    Some(format!(
        "TUN is not supported by v2ray; only apps using the SOCKS proxy at {} or the HTTP proxy at {} are proxied",
        listen_endpoint(&settings.listen_address, settings.socks_port),
        listen_endpoint(&settings.listen_address, settings.http_port),
    ))
}

fn listen_endpoint(addr: &str, port: u16) -> String {
    if addr.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{addr}]:{port}")
    } else {
        format!("{addr}:{port}")
    }
}

/// sing-box cannot program its strict IPv6 rules on a host whose kernel has
/// IPv6 disabled and fails to start; xray's strict routing is done by netctl,
/// which already skips the IPv6 rules there.
fn singbox_strict_route_allowed(backend: BackendType, host_has_ipv6: bool) -> bool {
    backend != BackendType::SingBox || host_has_ipv6
}

fn relays(state: &ProcessState) -> bool {
    matches!(
        state,
        ProcessState::Starting | ProcessState::Running | ProcessState::Stopping
    )
}

fn tun_session_for(rt: &TunRuntime) -> TunSession {
    TunSession {
        backend: rt.backend,
        iface: rt.iface.clone(),
    }
}

struct HealthProbe {
    proxy: SocketAddr,
    url: String,
    timeout: Duration,
}

/// Probes through the local HTTP proxy and reports health transitions. Its own
/// state receiver pauses probing while the manager respawns the backend and
/// restarts from a clean streak once it is running again.
fn spawn_health_monitor(
    probe: HealthProbe,
    timing: HealthTiming,
    mut state_rx: broadcast::Receiver<ProcessEvent>,
    sender: relm4::Sender<AppMsg>,
    generation: u64,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(timing.initial_delay).await;
        let mut tracker = HealthTracker::new();
        let mut tick = tokio::time::interval(timing.interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut paused = false;
        loop {
            tokio::select! {
                event = state_rx.recv() => match event {
                    Ok(ProcessEvent::StateChanged { to: ProcessState::Starting, .. }) => {
                        paused = true;
                        tracker.reset();
                    }
                    Ok(ProcessEvent::StateChanged { to: ProcessState::Running, .. }) => {
                        paused = false;
                        tracker.reset();
                        tick.reset();
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = tick.tick(), if !paused => {
                    let outcome = v2ray_rs_subscription::probe_via_http_proxy(
                        probe.proxy,
                        &probe.url,
                        probe.timeout,
                    )
                    .await;
                    if let Some(health) = tracker.record(outcome) {
                        sender.emit(AppMsg::ConnectionHealth(generation, health));
                    }
                }
            }
        }
    })
}

/// The new DNS failing mark when it differs from the last one reported.
fn dns_flip(window: &DnsFailureWindow, reported: &mut bool, now: Instant) -> Option<bool> {
    let failing = window.is_failing(now);
    if failing == *reported {
        return None;
    }
    *reported = failing;
    Some(failing)
}

/// Stops every task spawned for a candidate and waits until none can emit, so
/// nothing they relay can land after the terminal state that follows.
async fn halt(tasks: Vec<JoinHandle<()>>) {
    for task in &tasks {
        task.abort();
    }
    for task in tasks {
        let _ = task.await;
    }
}

/// Whether a host-level TUN failure is fixed by granting capabilities to the
/// backend and the route helper: the repair the grant prompt covers.
fn grant_fixable(e: &ProcessError) -> bool {
    matches!(
        e,
        ProcessError::TunCapabilityMissing(_) | ProcessError::TunHelperCapabilityMissing(_)
    )
}

/// A candidate's failure kept split: the summary prints `label: reason`, while
/// `key` is the normalized form the loop compares to spot a failure that every
/// candidate shares and that failing over cannot fix.
struct CandidateFailure {
    label: String,
    reason: String,
    key: String,
}

impl CandidateFailure {
    fn new(label: &str, reason: &str, address: &str, port: u16) -> Self {
        let reason = strip_ansi(reason);
        Self {
            key: failure_key(&reason, label, address, port),
            label: label.to_string(),
            reason,
        }
    }
}

fn record_failure(
    failures: &mut Vec<CandidateFailure>,
    failure: CandidateFailure,
    position: usize,
    total: usize,
) {
    log::info!(
        "candidate failed {position}/{total} label={} reason={}",
        failure.label,
        failure.reason
    );
    failures.push(failure);
}

/// Whether the last two candidates failed the same way.
fn repeats_previous(failures: &[CandidateFailure]) -> bool {
    match failures {
        [.., previous, last] => previous.key == last.key,
        _ => false,
    }
}

/// Drops CSI sequences so nothing the backend colored reaches the user.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.next_if_eq(&'[').is_none() {
            continue;
        }
        while chars
            .next_if(|c| ('\u{30}'..='\u{3f}').contains(c))
            .is_some()
        {}
        while chars
            .next_if(|c| ('\u{20}'..='\u{2f}').contains(c))
            .is_some()
        {}
        chars.next_if(|c| ('\u{40}'..='\u{7e}').contains(c));
    }
    out
}

/// The comparable form of a failure: everything that varies between candidates
/// purely because they are different attempts — colors, log timestamps, message
/// counters, the node's own name, address and port — collapses to a placeholder,
/// so two texts compare equal exactly when they describe the same failure.
fn failure_key(reason: &str, label: &str, address: &str, port: u16) -> String {
    let stripped = strip_ansi(reason);
    let mut key = mask_counters(strip_leading_timestamp(stripped.trim()));
    for token in [label, address] {
        // A two-letter remark such as "US" would mask unrelated substrings.
        if token.chars().count() >= 3 {
            key = key.replace(token, "<node>");
        }
    }
    mask_port(&key, port)
}

/// Removes one leading `YYYY/MM/DD HH:MM:SS(.frac)` stamp. Only the leading one
/// is an artifact of when the attempt ran; a date inside the text is content.
fn strip_leading_timestamp(text: &str) -> &str {
    let b = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        range
            .clone()
            .all(|i| b.get(i).is_some_and(u8::is_ascii_digit))
    };
    let literal = |i: usize, c: u8| b.get(i) == Some(&c);
    let shaped = digits(0..4)
        && literal(4, b'/')
        && digits(5..7)
        && literal(7, b'/')
        && digits(8..10)
        && literal(10, b' ')
        && digits(11..13)
        && literal(13, b':')
        && digits(14..16)
        && literal(16, b':')
        && digits(17..19);
    if !shaped {
        return text;
    }

    let mut end = 19;
    if literal(19, b'.') {
        let fraction = b
            .get(20..)
            .unwrap_or_default()
            .iter()
            .take(9)
            .take_while(|c| c.is_ascii_digit())
            .count();
        if fraction > 0 {
            end = 20 + fraction;
        }
    }
    while literal(end, b' ') {
        end += 1;
    }
    &text[end..]
}

/// Collapses bracketed all-digit tokens such as sing-box's `[0000]` message
/// counter. Named brackets like `[tun-in]` are content and stay.
fn mask_counters(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find(']') {
            Some(close) if close > 0 && after[..close].bytes().all(|c| c.is_ascii_digit()) => {
                out.push_str("[N]");
                rest = &after[close + 1..];
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Replaces the candidate's own port where the text reads it as a port: right
/// after a `:` and not running into another number. Requiring the colon keeps a
/// short port such as `80` out of prose like `80% packet loss`, and the trailing
/// check keeps `443` from being clipped out of `14430` or `10.4.43.1`.
fn mask_port(text: &str, port: u16) -> String {
    let needle = port.to_string();
    let bytes = text.as_bytes();
    let free = |c: Option<u8>| !c.is_some_and(|c| c.is_ascii_digit() || c == b'.');
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with(&needle)
            && i.checked_sub(1).map(|j| bytes[j]) == Some(b':')
            && free(bytes.get(i + needle.len()).copied())
        {
            out.push_str("<port>");
            i += needle.len();
            continue;
        }
        let c = text[i..].chars().next().expect("index on a char boundary");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// The message for the single terminal state. Two candidates that failed the
/// same way say nothing about any node, so a per-node list would only invite
/// another pointless retry.
fn terminal_failure(failures: &[CandidateFailure]) -> String {
    match failures.last() {
        Some(last) if repeats_previous(failures) => format!(
            "Connection failed on consecutive nodes with the same error (not node-specific): {}",
            last.reason
        ),
        _ => summarize_failures(failures),
    }
}

fn summarize_failures(failures: &[CandidateFailure]) -> String {
    if failures.is_empty() {
        return "All candidates failed".into();
    }

    let preview = failures
        .iter()
        .take(3)
        .map(|f| format!("{}: {}", f.label, f.reason))
        .collect::<Vec<_>>()
        .join("; ");

    if failures.len() > 3 {
        format!("All candidates failed: {preview}; ...")
    } else {
        format!("All candidates failed: {preview}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;
    use v2ray_rs_core::models::{
        DnsStrategy, ShadowsocksConfig, TransportSettings, TunConfig, VlessConfig, XhttpSettings,
    };
    use v2ray_rs_core::persistence::load_tun_session;
    use v2ray_rs_core::profile::AppProfile;

    use crate::health::Health;

    const GENERATION: u64 = 7;
    const RECV_TIMEOUT: Duration = Duration::from_secs(20);

    struct Stub {
        _tmp: tempfile::TempDir,
        paths: AppPaths,
        binary: PathBuf,
    }

    fn stub(script: &str) -> Stub {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_profile_in(AppProfile::Test, tmp.path());
        paths.ensure_dirs().unwrap();
        let binary = tmp.path().join("backend");
        std::fs::write(&binary, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        Stub {
            _tmp: tmp,
            paths,
            binary,
        }
    }

    fn executable(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn candidate(address: &str) -> ConnectionCandidate {
        ConnectionCandidate {
            node_ref: ConnectionNodeRef::Manual {
                node_id: uuid::Uuid::new_v4(),
            },
            source_name: "test".into(),
            node: node(address),
            latency_ms: None,
            real_delay_ms: None,
        }
    }

    fn connect(
        stub: &Stub,
        settings: AppSettings,
        candidates: Vec<ConnectionCandidate>,
    ) -> (ConnectionHandle, relm4::Receiver<AppMsg>) {
        connect_with(stub, settings, candidates, |mgr| mgr)
    }

    fn request(
        stub: &Stub,
        settings: AppSettings,
        candidates: Vec<ConnectionCandidate>,
    ) -> ConnectionRequest {
        ConnectionRequest {
            binary_path: stub.binary.clone(),
            candidates,
            writer: ConfigWriter::new(&settings, &stub.paths),
            paths: stub.paths.clone(),
            pid_path: stub.paths.pid_file_path(),
            geodata_dir: stub.paths.geodata_dir(),
            settings,
            enabled_rules: Vec::new(),
            subscriptions: Vec::new(),
            manual_nodes: Vec::new(),
            lifecycle: TunLifecycle::default(),
            generation: GENERATION,
            host_has_ipv6: true,
            health_timing: HealthTiming::default(),
        }
    }

    const FAST_HEALTH: HealthTiming = HealthTiming {
        initial_delay: Duration::from_millis(50),
        interval: Duration::from_millis(100),
        dns_recheck: Duration::from_millis(100),
    };

    fn connect_with_health(
        stub: &Stub,
        settings: AppSettings,
        candidates: Vec<ConnectionCandidate>,
        health_timing: HealthTiming,
    ) -> (ConnectionHandle, relm4::Receiver<AppMsg>) {
        let (tx, rx) = relm4::channel::<AppMsg>();
        let req = ConnectionRequest {
            health_timing,
            ..request(stub, settings, candidates)
        };
        let handle = spawn_with(req, tx, |mgr| mgr);
        (handle, rx)
    }

    fn connect_with(
        stub: &Stub,
        settings: AppSettings,
        candidates: Vec<ConnectionCandidate>,
        configure: impl Fn(ProcessManager) -> ProcessManager + Send + 'static,
    ) -> (ConnectionHandle, relm4::Receiver<AppMsg>) {
        let (tx, rx) = relm4::channel::<AppMsg>();
        let handle = spawn_with(request(stub, settings, candidates), tx, configure);
        (handle, rx)
    }

    fn singbox_settings() -> AppSettings {
        let mut settings = AppSettings::default();
        settings.backend.backend_type = BackendType::SingBox;
        settings
    }

    /// Settings whose SOCKS port is a live listener, so a stub backend passes
    /// the readiness probe. The listener must outlive the connection under test.
    fn ready_singbox_settings() -> (std::net::TcpListener, AppSettings) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut settings = singbox_settings();
        settings.socks_port = listener.local_addr().unwrap().port();
        (listener, settings)
    }

    async fn next_state(
        rx: &relm4::Receiver<AppMsg>,
    ) -> (ProcessState, Option<ConnectionMetadata>) {
        loop {
            let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
                .await
                .expect("no state reported in time")
                .expect("connection task ended without a terminal state");
            if let AppMsg::ProcessStateConnection(generation, state, connection) = msg {
                assert_eq!(generation, GENERATION);
                return (state, connection);
            }
        }
    }

    /// The task drops every sender when it returns, so a closed channel with
    /// no state in between proves nothing followed the terminal state.
    async fn assert_nothing_after_terminal(rx: &relm4::Receiver<AppMsg>) {
        loop {
            let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
                .await
                .expect("connection task outlived its terminal state");
            match msg {
                None => return,
                Some(AppMsg::ProcessStateConnection(_, state, _)) => {
                    panic!("{state:?} reported after the terminal state")
                }
                Some(_) => {}
            }
        }
    }

    #[test]
    fn forwarder_relays_only_nonterminal_states() {
        assert!(relays(&ProcessState::Starting));
        assert!(relays(&ProcessState::Running));
        assert!(relays(&ProcessState::Stopping));
        assert!(!relays(&ProcessState::Stopped));
        assert!(!relays(&ProcessState::Error("boom".into())));
    }

    #[test]
    fn failure_key_ignores_the_singbox_message_counter() {
        let first = failure_key(
            "\u{1b}[31mFATAL\u{1b}[0m[0000] start service: initialize inbound/tun[0]: configure tun interface: operation not permitted",
            "203.0.113.1",
            "203.0.113.1",
            8388,
        );
        let second = failure_key(
            "\u{1b}[31mFATAL\u{1b}[0m[0001] start service: initialize inbound/tun[0]: configure tun interface: operation not permitted",
            "203.0.113.2",
            "203.0.113.2",
            8388,
        );
        assert_eq!(first, second);
        assert!(!first.contains('\u{1b}'), "{first}");
    }

    #[test]
    fn failure_key_ignores_xray_timestamps() {
        let first = failure_key(
            "2026/09/14 10:37:35.309646 [Warning] failed to handle connection",
            "203.0.113.1",
            "203.0.113.1",
            443,
        );
        let second = failure_key(
            "2026/09/14 10:41:02.884131 [Warning] failed to handle connection",
            "203.0.113.2",
            "203.0.113.2",
            443,
        );
        assert_eq!(first, second);
        assert!(first.starts_with("[Warning]"), "{first}");
    }

    #[test]
    fn failure_key_ignores_each_candidates_own_host() {
        let first = failure_key(
            "tls: failed to verify certificate for 203.0.113.1:443",
            "203.0.113.1",
            "203.0.113.1",
            443,
        );
        let second = failure_key(
            "tls: failed to verify certificate for 203.0.113.2:443",
            "203.0.113.2",
            "203.0.113.2",
            443,
        );
        assert_eq!(first, second);
        assert_eq!(first, "tls: failed to verify certificate for <node>:<port>");
    }

    #[test]
    fn failure_key_keeps_unrelated_hosts_distinct() {
        let first = failure_key(
            "tls: handshake with updates.example.com failed",
            "203.0.113.1",
            "203.0.113.1",
            443,
        );
        let second = failure_key(
            "tls: handshake with mirror.example.net failed",
            "203.0.113.2",
            "203.0.113.2",
            443,
        );
        assert_ne!(first, second);
    }

    #[test]
    fn failure_key_keeps_a_port_inside_a_longer_number() {
        let key = failure_key("dial tcp 10.4.43.1:14430: refused", "node", "node", 443);
        assert_eq!(key, "dial tcp 10.4.43.1:14430: refused");
    }

    #[test]
    fn failure_key_keeps_a_bare_port_number_in_prose() {
        let first = failure_key(
            "upstream updates.example.com reports 80% packet loss",
            "203.0.113.1",
            "203.0.113.1",
            80,
        );
        let second = failure_key(
            "upstream mirror.example.net reports 80% packet loss",
            "203.0.113.2",
            "203.0.113.2",
            80,
        );
        assert!(first.contains("80% packet loss"), "{first}");
        assert_ne!(first, second);

        let addressed = failure_key(
            "dial tcp 203.0.113.1:80: refused",
            "node",
            "203.0.113.1",
            80,
        );
        assert_eq!(addressed, "dial tcp <node>:<port>: refused");
    }

    #[test]
    fn failure_key_keeps_a_midstring_timestamp() {
        let first = failure_key(
            "[Warning] handshake started at 2026/09/14 10:37:35 failed",
            "203.0.113.1",
            "203.0.113.1",
            443,
        );
        let second = failure_key(
            "[Warning] handshake started at 2026/09/14 10:41:02 failed",
            "203.0.113.2",
            "203.0.113.2",
            443,
        );
        assert!(first.contains("2026/09/14 10:37:35"), "{first}");
        assert_ne!(first, second);
    }

    #[test]
    fn summarize_failures_strips_ansi() {
        let summary = summarize_failures(&[CandidateFailure::new(
            "203.0.113.1",
            "\u{1b}[31mFATAL\u{1b}[0m[0000] configure tun interface: operation not permitted",
            "203.0.113.1",
            443,
        )]);
        assert!(!summary.contains('\u{1b}'), "{summary}");
        assert!(summary.contains("FATAL[0000]"), "{summary}");
    }

    #[test]
    fn repeats_previous_needs_two_equal_keys() {
        let failure = |label: &str, reason: &str| CandidateFailure::new(label, reason, label, 443);
        let shared =
            "\u{1b}[31mFATAL\u{1b}[0m[0000] configure tun interface: operation not permitted";

        assert!(!repeats_previous(&[]));
        assert!(!repeats_previous(&[failure("203.0.113.1", shared)]));
        assert!(repeats_previous(&[
            failure("203.0.113.1", shared),
            failure("203.0.113.2", shared),
        ]));
        assert!(!repeats_previous(&[
            failure("203.0.113.1", shared),
            failure("203.0.113.2", "config rejected"),
        ]));
    }

    #[test]
    fn strict_route_allowed_unless_singbox_lacks_ipv6() {
        let cases = [
            // (backend, host_has_ipv6, want)
            (BackendType::SingBox, false, false),
            (BackendType::SingBox, true, true),
            (BackendType::Xray, false, true),
            (BackendType::V2ray, false, true),
        ];
        for (backend, host_has_ipv6, want) in cases {
            assert_eq!(
                singbox_strict_route_allowed(backend, host_has_ipv6),
                want,
                "backend={backend:?} host_has_ipv6={host_has_ipv6}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_reason_reaches_the_exit_record() {
        let stub = stub(
            r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; exec sleep 30"#,
        );
        let (_listener, settings) = ready_singbox_settings();
        let (handle, rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);

        loop {
            let (state, _) = next_state(&rx).await;
            if matches!(state, ProcessState::Running) {
                break;
            }
            assert!(relays(&state), "start reported {state:?}");
        }

        handle.stop(StopReason::NodeSwitch);
        loop {
            let (state, _) = next_state(&rx).await;
            if matches!(state, ProcessState::Stopped) {
                break;
            }
            assert!(relays(&state), "stop reported {state:?}");
        }

        let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log")).unwrap();
        let exit = log
            .lines()
            .find(|l| l.contains(" exit requested="))
            .unwrap_or_else(|| panic!("no exit record in {log}"));
        assert!(exit.contains("reason=node-switch"), "{exit}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failover_reports_no_stopped_and_stop_reports_one() {
        let stub = stub(
            r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; grep -q 203.0.113.1 "$3" && exit 1; exec sleep 30"#,
        );
        let (_listener, settings) = ready_singbox_settings();
        let (handle, rx) = connect(
            &stub,
            settings,
            vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
        );

        loop {
            let (state, connection) = next_state(&rx).await;
            assert!(relays(&state), "failover reported {state:?}");
            if matches!(state, ProcessState::Running)
                && connection.is_some_and(|c| c.node_address == "203.0.113.2")
            {
                break;
            }
        }

        handle.stop(StopReason::UserStop);
        loop {
            let (state, _) = next_state(&rx).await;
            if matches!(state, ProcessState::Stopped) {
                break;
            }
            assert!(relays(&state), "stop reported {state:?}");
        }
        assert_nothing_after_terminal(&rx).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn last_candidate_failure_reports_one_error() {
        let stub = stub(
            r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && { grep -q 203.0.113.3 "$3" && exit 1; exit 0; }; grep -q 203.0.113.1 "$3" && exit 1; exec sleep 30"#,
        );
        let (_handle, rx) = connect(
            &stub,
            singbox_settings(),
            vec![candidate("203.0.113.1"), candidate("203.0.113.3")],
        );

        let msg = loop {
            match next_state(&rx).await {
                (ProcessState::Error(msg), _) => break msg,
                (state, _) => assert!(relays(&state), "failover reported {state:?}"),
            }
        };
        assert!(msg.starts_with("All candidates failed"), "{msg}");
        assert!(
            msg.contains("203.0.113.1: process exited with code 1"),
            "{msg}"
        );
        assert!(msg.contains("203.0.113.3: config rejected"), "{msg}");
        assert_nothing_after_terminal(&rx).await;
    }

    /// Lines the stub appended, one per candidate it was asked to check.
    fn attempts(marker: &std::path::Path) -> usize {
        std::fs::read_to_string(marker)
            .unwrap_or_default()
            .lines()
            .count()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn same_failure_on_consecutive_candidates_stops_failover() {
        let launches = tempfile::tempdir().unwrap();
        let marker = launches.path().join("launched");
        let stub = stub(&format!(
            r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0
[ "$1" = check ] || exec sleep 30
echo attempt >> {}
printf '\033[31mFATAL\033[0m[0000] configure tun interface: operation not permitted\n' >&2
exit 1"#,
            marker.display()
        ));
        let (_handle, rx) = connect(
            &stub,
            singbox_settings(),
            vec![
                candidate("203.0.113.1"),
                candidate("203.0.113.2"),
                candidate("203.0.113.3"),
            ],
        );

        let (terminal, _) = drain(&rx).await;
        let Some(ProcessState::Error(msg)) = terminal else {
            panic!("expected a single error terminal, got {terminal:?}");
        };
        assert!(
            msg.starts_with("Connection failed on consecutive nodes with the same error"),
            "{msg}"
        );
        assert!(
            msg.contains("configure tun interface: operation not permitted"),
            "{msg}"
        );
        assert!(!msg.contains('\u{1b}'), "{msg}");
        assert_eq!(attempts(&marker), 2);

        let config = std::fs::read_to_string(stub.paths.generated_dir().join("sing-box.json"))
            .expect("generated config readable");
        assert!(config.contains("203.0.113.2"), "{config}");
        assert!(
            !config.contains("203.0.113.3"),
            "third candidate must not be attempted: {config}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn differing_failures_keep_failing_over() {
        let launches = tempfile::tempdir().unwrap();
        let marker = launches.path().join("launched");
        let stub = stub(&format!(
            r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0
[ "$1" = check ] || exec sleep 30
echo attempt >> {}
grep -q 203.0.113.1 "$3" && echo "alpha refused" >&2 && exit 1
grep -q 203.0.113.2 "$3" && echo "bravo refused" >&2 && exit 1
echo "charlie refused" >&2
exit 1"#,
            marker.display()
        ));
        let (_handle, rx) = connect(
            &stub,
            singbox_settings(),
            vec![
                candidate("203.0.113.1"),
                candidate("203.0.113.2"),
                candidate("203.0.113.3"),
            ],
        );

        let (terminal, _) = drain(&rx).await;
        let Some(ProcessState::Error(msg)) = terminal else {
            panic!("expected a single error terminal, got {terminal:?}");
        };
        assert!(msg.starts_with("All candidates failed"), "{msg}");
        for reason in ["alpha refused", "bravo refused", "charlie refused"] {
            assert!(msg.contains(reason), "{msg}");
        }
        assert!(!msg.contains('\u{1b}'), "{msg}");
        assert_eq!(attempts(&marker), 3);
    }

    /// Drains the connection until its channel closes, returning the terminal
    /// state and every log line it emitted.
    async fn drain(rx: &relm4::Receiver<AppMsg>) -> (Option<ProcessState>, Vec<String>) {
        let mut terminal = None;
        let mut lines = Vec::new();
        loop {
            let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
                .await
                .expect("connection task outlived its terminal state");
            match msg {
                None => return (terminal, lines),
                Some(AppMsg::ProcessLogLine(generation, line)) => {
                    assert_eq!(generation, GENERATION);
                    lines.push(line);
                }
                Some(AppMsg::ProcessStateConnection(generation, state, _)) => {
                    assert_eq!(generation, GENERATION);
                    if !relays(&state) {
                        assert!(terminal.is_none(), "{state:?} after the terminal state");
                        terminal = Some(state);
                    }
                }
                Some(_) => {}
            }
        }
    }

    fn strict_route_stub() -> Stub {
        stub(
            r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; exec sleep 30"#,
        )
    }

    fn strict_route_settings() -> AppSettings {
        let mut settings = tun_settings();
        settings.backend.backend_type = BackendType::SingBox;
        settings.tun.interface_name = "v2rstest2".into();
        settings
    }

    fn capless_probe(mgr: ProcessManager) -> ProcessManager {
        mgr.with_host_probe(v2ray_rs_process::HostProbe {
            getcap: PathBuf::from("/bin/true"),
            helper: PathBuf::from("/bin/true"),
        })
    }

    fn xhttp_candidate(address: &str) -> ConnectionCandidate {
        ConnectionCandidate {
            node: ProxyNode::Vless(VlessConfig {
                address: address.into(),
                port: 443,
                uuid: "b831381d-6324-4d53-ad4f-8cda48b30811".into(),
                encryption: None,
                flow: None,
                transport: TransportSettings::Xhttp(XhttpSettings {
                    path: "/x".into(),
                    host: None,
                    mode: "auto".into(),
                }),
                tls: None,
                remark: None,
            }),
            ..candidate(address)
        }
    }

    fn assert_error_terminal(terminal: Option<ProcessState>) {
        assert!(
            matches!(terminal, Some(ProcessState::Error(_))),
            "expected an error terminal, got {terminal:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn singbox_tun_without_ipv6_turns_strict_route_off_once() {
        let stub = strict_route_stub();
        let settings = strict_route_settings();
        assert!(settings.tun.strict_route);
        let mut req = request(
            &stub,
            settings,
            vec![xhttp_candidate("203.0.113.1"), candidate("203.0.113.2")],
        );
        req.host_has_ipv6 = false;
        let (tx, rx) = relm4::channel::<AppMsg>();
        let _handle = spawn_with(req, tx, capless_probe);

        let (terminal, lines) = drain(&rx).await;
        assert_error_terminal(terminal);

        let config = std::fs::read_to_string(stub.paths.generated_dir().join("sing-box.json"))
            .expect("generated config readable");
        assert!(config.contains("203.0.113.2"), "{config}");
        assert!(config.contains(r#""strict_route":false"#), "{config}");

        let notices = lines.iter().filter(|l| *l == STRICT_ROUTE_NOTICE).count();
        assert_eq!(notices, 1, "{lines:?}");
        let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
            .expect("backend.log readable");
        assert_eq!(log.matches(STRICT_ROUTE_NOTICE).count(), 1, "{log}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn singbox_tun_with_ipv6_keeps_strict_route() {
        let stub = strict_route_stub();
        let settings = strict_route_settings();
        let mut req = request(&stub, settings, vec![candidate("203.0.113.2")]);
        req.host_has_ipv6 = true;
        let (tx, rx) = relm4::channel::<AppMsg>();
        let _handle = spawn_with(req, tx, capless_probe);

        let (terminal, lines) = drain(&rx).await;
        assert_error_terminal(terminal);

        let config = std::fs::read_to_string(stub.paths.generated_dir().join("sing-box.json"))
            .expect("generated config readable");
        assert!(config.contains(r#""strict_route":true"#), "{config}");
        assert!(!lines.iter().any(|l| l == STRICT_ROUTE_NOTICE), "{lines:?}");
        let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
            .expect("backend.log readable");
        assert!(!log.contains(STRICT_ROUTE_NOTICE), "{log}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn v2ray_tun_warning_logged_once_per_connection() {
        let stub = stub(r#"exit 1"#);
        let settings = v2ray_settings(2080, 2081, "127.0.0.1", true);
        let expected = v2ray_tun_warning(&settings).expect("warning");
        assert!(expected.contains("127.0.0.1:2080"), "{expected}");
        assert!(expected.contains("127.0.0.1:2081"), "{expected}");
        let req = request(
            &stub,
            settings,
            vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
        );
        let (tx, rx) = relm4::channel::<AppMsg>();
        let _handle = spawn_with(req, tx, capless_probe);

        let (terminal, lines) = drain(&rx).await;
        assert_error_terminal(terminal);

        assert_eq!(
            lines.iter().filter(|l| *l == &expected).count(),
            1,
            "{lines:?}"
        );
        let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
            .expect("backend.log readable");
        assert_eq!(log.matches(&expected).count(), 1, "{log}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn v2ray_without_tun_logs_no_warning() {
        let stub = stub(r#"exit 1"#);
        let expected =
            v2ray_tun_warning(&v2ray_settings(2080, 2081, "127.0.0.1", true)).expect("warning");
        let req = request(
            &stub,
            v2ray_settings(2080, 2081, "127.0.0.1", false),
            vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
        );
        let (tx, rx) = relm4::channel::<AppMsg>();
        let _handle = spawn_with(req, tx, capless_probe);

        let (terminal, lines) = drain(&rx).await;
        assert_error_terminal(terminal);

        assert!(!lines.iter().any(|l| l.contains(&expected)), "{lines:?}");
        let log = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
            .expect("backend.log readable");
        assert!(!log.contains(&expected), "{log}");
    }

    #[test]
    fn grant_fixable_is_exactly_the_capability_pair() {
        let fixable = [
            ProcessError::TunCapabilityMissing(PathBuf::from("/usr/bin/xray")),
            ProcessError::TunHelperCapabilityMissing(PathBuf::from(
                "/usr/local/bin/v2ray-rs-netctl",
            )),
        ];
        assert!(fixable.iter().all(grant_fixable));

        let other_host_level = [
            ProcessError::TunCapabilityProbe("getcap not installed".into()),
            ProcessError::TunMountUnsupported("/dev/net/tun: required key not available".into()),
            ProcessError::TunHelperMissing,
            ProcessError::TunHelperRelogin,
            ProcessError::BackendTooOld {
                backend: BackendType::Xray,
                installed: "25.3.5".into(),
                required: "26.1.13".into(),
            },
        ];
        assert!(other_host_level.iter().all(|e| !grant_fixable(e)));
        assert!(!grant_fixable(&ProcessError::ConfigCheck(
            "rejected".into()
        )));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_level_failure_stops_the_candidate_loop() {
        let stub = stub(r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; exit 1"#);
        // A getcap that prints nothing is the "no capabilities" verdict, and a
        // present helper gets the start past the helper gates to that verdict.
        // `/bin/true` rather than a freshly written script: exec'ing a file a
        // parallel test may still hold open for writing fails with ETXTBSY.
        let host = tempfile::tempdir().unwrap();
        let probe = v2ray_rs_process::HostProbe {
            getcap: PathBuf::from("/bin/true"),
            helper: executable(host.path(), "v2ray-rs-netctl"),
        };
        let mut settings = tun_settings();
        settings.backend.backend_type = BackendType::Xray;
        settings.tun.interface_name = "v2rstest1".into();
        let (_handle, rx) = connect_with(
            &stub,
            settings,
            vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
            move |mgr| mgr.with_host_probe(probe.clone()),
        );

        let mut granted = false;
        let terminal = loop {
            let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
                .await
                .expect("no message in time")
                .expect("connection task ended without a terminal state");
            match msg {
                AppMsg::TunGrantRequired(generation) => {
                    assert_eq!(generation, GENERATION);
                    granted = true;
                }
                AppMsg::ProcessStateConnection(generation, state, _) => {
                    assert_eq!(generation, GENERATION);
                    break state;
                }
                _ => {}
            }
        };

        let ProcessState::Error(msg) = terminal else {
            panic!("expected the host-level gate to stop the loop, got {terminal:?}");
        };
        assert!(
            !msg.starts_with("All candidates failed"),
            "a host-level failure is reported on its own, not summarized: {msg}"
        );
        assert!(msg.contains("lacks CAP_NET_ADMIN"), "{msg}");
        assert!(
            granted,
            "grant-fixable failure must emit TunGrantRequired first: {msg}"
        );
        // Exactly one start attempt: the config on disk is still candidate
        // 1's; a second attempt would have overwritten it with candidate 2.
        let config = std::fs::read_to_string(stub.paths.generated_dir().join("xray.json"))
            .expect("generated config readable");
        assert!(config.contains("203.0.113.1"), "{config}");
        assert!(
            !config.contains("203.0.113.2"),
            "second candidate must not be attempted: {config}"
        );
        assert_nothing_after_terminal(&rx).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_lines_carry_connection_generation() {
        let stub = stub(
            r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; while :; do echo v2rs-log-line; sleep 0.2; done"#,
        );
        let (_listener, settings) = ready_singbox_settings();
        let (handle, rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);

        loop {
            let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
                .await
                .expect("no log line in time")
                .expect("connection task ended before a log line");
            if let AppMsg::ProcessLogLine(generation, line) = msg {
                assert_eq!(generation, GENERATION);
                assert_eq!(line, "v2rs-log-line");
                break;
            }
        }

        handle.stop(StopReason::UserStop);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn marker_written_from_runtime_before_start() {
        let stub = stub(r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; exit 1"#);
        let mut settings = tun_settings();
        settings.backend.backend_type = BackendType::Xray;
        settings.tun.interface_name = "v2rstest0".into();
        let (_handle, rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);

        let (state, _) = next_state(&rx).await;
        let ProcessState::Error(msg) = state else {
            panic!("expected the capability gate to fail the start, got {state:?}");
        };
        assert!(
            msg.contains("CAP_NET_ADMIN")
                || msg.contains("TUN capabilities")
                || msg.contains("ignores file capabilities"),
            "start should fail at the capability gate: {msg}"
        );
        assert_eq!(
            load_tun_session(&stub.paths),
            Some(TunSession {
                backend: BackendType::Xray,
                iface: "v2rstest0".into(),
            })
        );
        assert_nothing_after_terminal(&rx).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_marker_when_launched_without_tun() {
        let stub = stub("exit 1");
        let (_handle, rx) = connect(&stub, singbox_settings(), vec![candidate("203.0.113.1")]);

        loop {
            match next_state(&rx).await {
                (ProcessState::Error(_), _) => break,
                (state, _) => assert!(relays(&state), "reported {state:?}"),
            }
        }
        assert_eq!(load_tun_session(&stub.paths), None);
        assert_nothing_after_terminal(&rx).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn live_connect_writes_backend_diagnostics() {
        let stub = stub(
            r#"[ "$1" = version ] && echo "sing-box 1.13.0" && exit 0; [ "$1" = check ] && exit 0; exec sleep 30"#,
        );
        let (_listener, settings) = ready_singbox_settings();
        let (handle, rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);

        loop {
            let (state, _) = next_state(&rx).await;
            if matches!(state, ProcessState::Running) {
                break;
            }
            assert!(relays(&state), "reported {state:?}");
        }

        let contents = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
            .expect("backend.log readable");
        let session_at = contents.find(" session ").expect("session record");
        assert_eq!(contents.matches(" session ").count(), 1, "{contents}");
        assert!(
            contents.contains("backend=sing-box version=1.13.0 node=203.0.113.1 tun=off"),
            "{contents}"
        );
        let session_line = contents[session_at..].lines().next().unwrap_or_default();
        assert!(
            session_line.ends_with(
                "tun=off hijack=off capture_dns=false strict=false nodes_pinned=true profile=app"
            ),
            "{session_line}"
        );

        handle.stop(StopReason::UserStop);
        loop {
            let (state, _) = next_state(&rx).await;
            if matches!(state, ProcessState::Stopped) {
                break;
            }
            assert!(relays(&state), "stop reported {state:?}");
        }

        let contents = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
            .expect("backend.log readable");
        let exit_at = contents.find(" exit ").expect("exit record");
        assert_eq!(contents.matches(" exit ").count(), 1, "{contents}");
        assert!(contents[exit_at..].contains("requested=true"), "{contents}");
        assert!(session_at < exit_at, "{contents}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn xray_tun_session_record_carries_dns_decisions() {
        let stub = stub(
            r#"[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && exit 0; exec sleep 30"#,
        );
        let host = tempfile::tempdir().unwrap();
        let getcap = host.path().join("getcap");
        std::fs::write(&getcap, "#!/bin/sh\necho \"$1 cap_net_admin=ep\"\n").unwrap();
        std::fs::set_permissions(&getcap, std::fs::Permissions::from_mode(0o755)).unwrap();
        let probe = v2ray_rs_process::HostProbe {
            getcap,
            helper: executable(host.path(), "v2ray-rs-netctl"),
        };
        let mut settings = tun_settings();
        settings.backend.backend_type = BackendType::Xray;
        settings.tun.interface_name = "v2rstest3".into();
        settings.tun.strict_route = false;
        let (_handle, rx) = connect_with(
            &stub,
            settings,
            vec![candidate("203.0.113.1")],
            move |mgr| mgr.with_host_probe(probe.clone()),
        );

        let (terminal, _) = drain(&rx).await;
        assert_error_terminal(terminal);

        let contents = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
            .expect("backend.log readable");
        assert_eq!(contents.matches(" session ").count(), 1, "{contents}");
        assert!(
            contents.contains(
                "tun=on hijack=hijack capture_dns=true strict=false nodes_pinned=true profile=app"
            ),
            "{contents}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failover_is_traceable() {
        let capture = crate::logging::install_test_capture();
        let stub = stub(
            r#"[ "$1" = version ] && echo "sing-box 1.13.0" && exit 0; [ "$1" = check ] && exit 0; grep -q 198.51.100.11 "$3" && { echo "FATAL start service: trace refused" >&2; exit 1; }; exec sleep 30"#,
        );
        let (_listener, settings) = ready_singbox_settings();
        let (handle, rx) = connect(
            &stub,
            settings,
            vec![
                candidate("198.51.100.11"),
                candidate("198.51.100.12"),
                candidate("198.51.100.13"),
            ],
        );

        loop {
            let (state, _) = next_state(&rx).await;
            assert!(relays(&state), "failover reported {state:?}");
            if matches!(state, ProcessState::Running) {
                break;
            }
        }
        handle.stop(StopReason::UserStop);

        let lines = capture.lines_containing("198.51.100.1");
        let at = |prefix: &str, address: &str| {
            lines
                .iter()
                .position(|line| line.starts_with(prefix) && line.contains(address))
                .unwrap_or_else(|| panic!("no {prefix:?} line for {address}: {lines:#?}"))
        };
        let started = at("candidate start 1/3 ", "198.51.100.11");
        let failed = at("candidate failed 1/3 ", "198.51.100.11");
        let next = at("candidate start 2/3 ", "198.51.100.12");
        assert!(started < failed && failed < next, "{lines:#?}");
        assert!(lines[failed].contains("reason="), "{}", lines[failed]);
        assert!(lines[failed].contains("trace refused"), "{}", lines[failed]);
        assert!(
            !lines.iter().any(|line| line.contains("198.51.100.13")),
            "{lines:#?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn two_candidates_first_exits_before_ready() {
        let stub = stub(
            r#"[ "$1" = version ] && echo "sing-box 1.13.0" && exit 0; [ "$1" = check ] && exit 0; grep -q 203.0.113.1 "$3" && { echo "FATAL start service: listen failed" >&2; exit 1; }; exec sleep 30"#,
        );
        let (_listener, settings) = ready_singbox_settings();
        let (handle, rx) = connect(
            &stub,
            settings,
            vec![candidate("203.0.113.1"), candidate("203.0.113.2")],
        );

        loop {
            let (state, connection) = next_state(&rx).await;
            assert!(relays(&state), "failover reported {state:?}");
            if matches!(state, ProcessState::Running) {
                let address = connection.map(|c| c.node_address);
                assert_eq!(address.as_deref(), Some("203.0.113.2"));
                break;
            }
        }

        let contents = std::fs::read_to_string(stub.paths.logs_dir().join("backend.log"))
            .expect("backend.log readable");
        assert_eq!(contents.matches(" session ").count(), 2, "{contents}");
        let exit_at = contents.find(" exit ").expect("exit record");
        assert_eq!(
            contents[..exit_at].matches(" session ").count(),
            1,
            "{contents}"
        );
        assert!(
            contents[..exit_at].contains("node=203.0.113.1"),
            "{contents}"
        );

        handle.stop(StopReason::UserStop);
        loop {
            let (state, _) = next_state(&rx).await;
            if matches!(state, ProcessState::Stopped) {
                break;
            }
            assert!(relays(&state), "stop reported {state:?}");
        }
        assert_nothing_after_terminal(&rx).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_halts_every_forwarder() {
        let stub = stub(
            r#"[ "$1" = version ] && echo "sing-box 1.13.0" && exit 0; [ "$1" = check ] && exit 0; while :; do echo v2rs-log-line; sleep 0.05; done"#,
        );
        let (_listener, settings) = ready_singbox_settings();
        let (handle, rx) = connect(&stub, settings, vec![candidate("203.0.113.1")]);

        loop {
            let (state, _) = next_state(&rx).await;
            if matches!(state, ProcessState::Running) {
                break;
            }
            assert!(relays(&state), "reported {state:?}");
        }

        handle.stop(StopReason::UserStop);
        loop {
            let (state, _) = next_state(&rx).await;
            if matches!(state, ProcessState::Stopped) {
                break;
            }
            assert!(relays(&state), "stop reported {state:?}");
        }
        loop {
            let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
                .await
                .expect("connection task outlived its terminal state");
            match msg {
                None => break,
                Some(AppMsg::ProcessLogLine(_, line)) => {
                    panic!("log line {line:?} relayed after the terminal state")
                }
                Some(AppMsg::ProcessStateConnection(_, state, _)) => {
                    panic!("{state:?} reported after the terminal state")
                }
                Some(_) => {}
            }
        }
    }

    struct FakeProxy {
        port: u16,
        accepts: Arc<std::sync::atomic::AtomicUsize>,
        healthy: Arc<std::sync::atomic::AtomicBool>,
    }

    impl FakeProxy {
        fn accepts(&self) -> usize {
            self.accepts.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn set_healthy(&self, healthy: bool) {
            self.healthy
                .store(healthy, std::sync::atomic::Ordering::SeqCst);
        }
    }

    async fn fake_proxy(healthy: bool) -> FakeProxy {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = FakeProxy {
            port: listener.local_addr().unwrap().port(),
            accepts: Arc::default(),
            healthy: Arc::new(std::sync::atomic::AtomicBool::new(healthy)),
        };
        let accepts = proxy.accepts.clone();
        let healthy = proxy.healthy.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                accepts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let reply = if healthy.load(std::sync::atomic::Ordering::SeqCst) {
                    "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = socket.read(&mut buf).await;
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        proxy
    }

    fn health_settings(proxy: &FakeProxy) -> (std::net::TcpListener, AppSettings) {
        let (listener, mut settings) = ready_singbox_settings();
        settings.http_port = proxy.port;
        settings.real_delay.test_url = "http://probe.test/generate_204".into();
        (listener, settings)
    }

    fn sleeping_stub() -> Stub {
        stub(
            r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; exec sleep 30"#,
        )
    }

    async fn wait_running(rx: &relm4::Receiver<AppMsg>) {
        loop {
            let (state, _) = next_state(rx).await;
            if matches!(state, ProcessState::Running) {
                return;
            }
            assert!(relays(&state), "reported {state:?}");
        }
    }

    async fn next_health(rx: &relm4::Receiver<AppMsg>) -> Health {
        loop {
            let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
                .await
                .expect("no health reported in time")
                .expect("connection task ended before a health report");
            if let AppMsg::ConnectionHealth(generation, health) = msg {
                assert_eq!(generation, GENERATION);
                return health;
            }
        }
    }

    async fn stop_and_wait(handle: &ConnectionHandle, rx: &relm4::Receiver<AppMsg>) {
        handle.stop(StopReason::UserStop);
        loop {
            let (state, _) = next_state(rx).await;
            if matches!(state, ProcessState::Stopped) {
                return;
            }
            assert!(relays(&state), "stop reported {state:?}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_monitor_reports_unhealthy_then_healthy() {
        let stub = sleeping_stub();
        let proxy = fake_proxy(false).await;
        let (_listener, settings) = health_settings(&proxy);
        let (handle, rx) =
            connect_with_health(&stub, settings, vec![candidate("203.0.113.1")], FAST_HEALTH);
        wait_running(&rx).await;

        let health = next_health(&rx).await;
        assert!(
            matches!(health, Health::Unhealthy(ref r) if r.contains("502")),
            "{health:?}"
        );
        proxy.set_healthy(true);
        assert_eq!(next_health(&rx).await, Health::Healthy);

        let seen = proxy.accepts();
        tokio::time::timeout(RECV_TIMEOUT, async {
            while proxy.accepts() < seen + 2 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("health probes stopped after recovery");
        handle.stop(StopReason::UserStop);
        loop {
            let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
                .await
                .expect("connection task outlived its terminal state");
            match msg {
                None => break,
                Some(AppMsg::ConnectionHealth(_, health)) => {
                    panic!("repeated health report {health:?}")
                }
                Some(_) => {}
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_monitor_is_not_spawned_when_disabled() {
        let stub = sleeping_stub();
        let proxy = fake_proxy(true).await;
        let (_listener, mut settings) = health_settings(&proxy);
        settings.health_check.enabled = false;
        let (handle, rx) =
            connect_with_health(&stub, settings, vec![candidate("203.0.113.1")], FAST_HEALTH);
        wait_running(&rx).await;

        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(proxy.accepts(), 0);

        stop_and_wait(&handle, &rx).await;
        assert_nothing_after_terminal(&rx).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_monitor_stops_with_the_connection() {
        let stub = sleeping_stub();
        let proxy = fake_proxy(true).await;
        let (_listener, settings) = health_settings(&proxy);
        let (handle, rx) =
            connect_with_health(&stub, settings, vec![candidate("203.0.113.1")], FAST_HEALTH);
        wait_running(&rx).await;
        assert_eq!(next_health(&rx).await, Health::Healthy);

        stop_and_wait(&handle, &rx).await;
        assert_nothing_after_terminal(&rx).await;
        let after_stop = proxy.accepts();
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(proxy.accepts(), after_stop, "probe sent after stop");
    }

    const DNS_BURST: &str = r#"sleep 2; i=0; while [ $i -lt 25 ]; do echo "2026/09/16 12:00:00 [Info] app/dns: failed to retrieve response for example.com"; i=$((i+1)); done; echo dns-burst-done; exec sleep 30"#;

    fn dns_burst_stub(version: &str) -> Stub {
        stub(&format!(
            r#"[ "$1" = version ] && echo "{version}" && exit 0; [ "$1" = check ] && exit 0; [ "$2" = -test ] && exit 0; {DNS_BURST}"#
        ))
    }

    /// Stops the connection once the burst is relayed and returns every DNS
    /// health report it emitted.
    async fn dns_reports(handle: &ConnectionHandle, rx: &relm4::Receiver<AppMsg>) -> Vec<bool> {
        let mut reports = Vec::new();
        loop {
            let msg = tokio::time::timeout(RECV_TIMEOUT, rx.recv())
                .await
                .expect("connection task outlived its terminal state");
            match msg {
                None => return reports,
                Some(AppMsg::DnsHealth(generation, failing)) => {
                    assert_eq!(generation, GENERATION);
                    reports.push(failing);
                }
                Some(AppMsg::ProcessLogLine(_, line)) if line == "dns-burst-done" => {
                    handle.stop(StopReason::UserStop)
                }
                Some(_) => {}
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn xray_dns_burst_reports_dns_health() {
        let stub = dns_burst_stub("Xray 26.6.27");
        let (_listener, mut settings) = ready_singbox_settings();
        settings.backend.backend_type = BackendType::Xray;
        settings.health_check.enabled = false;
        let (handle, rx) =
            connect_with_health(&stub, settings, vec![candidate("203.0.113.1")], FAST_HEALTH);

        assert_eq!(dns_reports(&handle, &rx).await, vec![true]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn singbox_dns_burst_reports_nothing() {
        let stub = dns_burst_stub("sing-box version 1.13.0");
        let (_listener, mut settings) = ready_singbox_settings();
        settings.health_check.enabled = false;
        let (handle, rx) =
            connect_with_health(&stub, settings, vec![candidate("203.0.113.1")], FAST_HEALTH);

        assert_eq!(dns_reports(&handle, &rx).await, Vec::<bool>::new());
    }

    #[test]
    fn dns_window_clears_after_the_recheck_tick() {
        let base = Instant::now();
        let mut window = DnsFailureWindow::default();
        let mut reported = false;
        for k in 0..20 {
            window.observe(
                "[Info] app/dns: failed to retrieve response for example.com",
                base + Duration::from_secs(k),
            );
        }
        let last = base + Duration::from_secs(19);
        assert_eq!(dns_flip(&window, &mut reported, last), Some(true));
        assert_eq!(dns_flip(&window, &mut reported, last), None);
        let tick = last + Duration::from_secs(61);
        assert_eq!(dns_flip(&window, &mut reported, tick), Some(false));
        assert_eq!(dns_flip(&window, &mut reported, tick), None);
    }

    fn node(address: &str) -> ProxyNode {
        ProxyNode::Shadowsocks(ShadowsocksConfig {
            address: address.into(),
            port: 8388,
            method: "aes-256-gcm".into(),
            password: "secret".into(),
            remark: None,
        })
    }

    fn tun_settings() -> AppSettings {
        AppSettings {
            tun: TunConfig {
                enabled: true,
                ..TunConfig::default()
            },
            ..AppSettings::default()
        }
    }

    fn v2ray_settings(socks: u16, http: u16, listen: &str, tun: bool) -> AppSettings {
        let mut settings = AppSettings {
            tun: TunConfig {
                enabled: tun,
                ..TunConfig::default()
            },
            ..AppSettings::default()
        };
        settings.backend.backend_type = BackendType::V2ray;
        settings.listen_address = listen.to_string();
        settings.socks_port = socks;
        settings.http_port = http;
        settings
    }

    #[test]
    fn v2ray_with_tun_warns_with_both_endpoints() {
        let warning = v2ray_tun_warning(&v2ray_settings(2080, 2081, "127.0.0.1", true))
            .expect("v2ray with TUN enabled must warn");

        assert!(warning.contains("127.0.0.1:2080"), "{warning}");
        assert!(warning.contains("127.0.0.1:2081"), "{warning}");
        assert!(warning.contains("v2ray"), "{warning}");
    }

    #[test]
    fn v2ray_without_tun_does_not_warn() {
        assert!(v2ray_tun_warning(&v2ray_settings(2080, 2081, "127.0.0.1", false)).is_none());
    }

    #[test]
    fn singbox_and_xray_with_tun_do_not_warn() {
        for backend in [BackendType::SingBox, BackendType::Xray] {
            let mut settings = v2ray_settings(2080, 2081, "127.0.0.1", true);
            settings.backend.backend_type = backend;

            assert!(v2ray_tun_warning(&settings).is_none(), "{backend:?}");
        }
    }

    #[test]
    fn ipv6_listen_address_is_bracketed() {
        let warning = v2ray_tun_warning(&v2ray_settings(2080, 2081, "::1", true))
            .expect("v2ray with TUN enabled must warn");

        assert!(warning.contains("[::1]:2080"), "{warning}");
        assert!(warning.contains("[::1]:2081"), "{warning}");
        assert!(!warning.contains("::1:2080"), "{warning}");
    }

    #[test]
    fn ip_addressed_nodes_need_no_pin() {
        let settings = tun_settings();
        assert!(hosts_cover_nodes(&settings, &[node("203.0.113.9")]));
    }

    #[test]
    fn hostname_without_a_pin_leaves_capture_off() {
        let mut settings = tun_settings();
        settings.backend.backend_type = BackendType::Xray;

        assert!(!hosts_cover_nodes(&settings, &[node("proxy.example.com")]));
        let runtime = build_tun_runtime(
            &settings,
            hosts_cover_nodes(&settings, &[node("proxy.example.com")]),
        )
        .unwrap();
        assert!(!runtime.capture_dns);
    }

    #[test]
    fn pinned_hostname_arms_capture() {
        let mut settings = tun_settings();
        settings.backend.backend_type = BackendType::Xray;
        settings.dns.hosts.push(HostOverride {
            domain: "proxy.example.com".into(),
            ip: "203.0.113.9".into(),
        });

        let nodes = [node("proxy.example.com")];
        assert!(hosts_cover_nodes(&settings, &nodes));
        let runtime = build_tun_runtime(&settings, hosts_cover_nodes(&settings, &nodes)).unwrap();
        assert!(runtime.capture_dns);
    }

    #[test]
    fn wrong_family_pin_leaves_capture_off() {
        let mut settings = tun_settings();
        settings.backend.backend_type = BackendType::Xray;
        settings.dns.strategy = DnsStrategy::Ipv4Only;
        settings.dns.hosts.push(HostOverride {
            domain: "proxy.example.com".into(),
            ip: "2001:db8::1".into(),
        });

        let nodes = [node("proxy.example.com")];
        assert!(
            !hosts_cover_nodes(&settings, &nodes),
            "an override of the wrong family is an empty answer, not a pin"
        );
    }
}
