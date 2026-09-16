use std::path::PathBuf;

use std::sync::Arc;

use tokio::sync::{Mutex, broadcast, mpsc};
use tokio::task::JoinHandle;
use v2ray_rs_core::config::ConfigWriter;
use v2ray_rs_core::models::{
    AppSettings, BackendType, ConnectionMetadata, ConnectionNodeRef, DnsHijackMode, HostOverride,
    ManualNode, ProxyNode, RoutingRule, Subscription, resolve_effective_config,
};
use v2ray_rs_core::persistence::{AppPaths, TunSession, save_tun_session};
use v2ray_rs_core::resolve::{ConnectionCandidate, resolve_via_nodes};
use v2ray_rs_core::rotating_log::{DEFAULT_MAX_BYTES, RotatingFileWriter};
use v2ray_rs_process::{ProcessError, ProcessEvent, ProcessManager, ProcessState, TunRuntime};

use crate::app::AppMsg;

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
}

enum ConnectionCmd {
    Stop,
}

impl ConnectionHandle {
    pub(super) fn stop(&self) {
        let _ = self.cmd_tx.try_send(ConnectionCmd::Stop);
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
        if matches!(cmd_rx.try_recv(), Ok(ConnectionCmd::Stop)) {
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

        'candidates: for candidate in candidates {
            if matches!(cmd_rx.try_recv(), Ok(ConnectionCmd::Stop)) {
                if let Some(mut failed) = parked.take() {
                    failed.shutdown().await;
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
                        failures.push(CandidateFailure::new(
                            &candidate_label,
                            &format!("config generation failed: {e}"),
                            &candidate_address,
                            candidate_port,
                        ));
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
                .with_log_file(backend_log.clone()),
            );

            let started = tokio::select! {
                biased;
                Some(ConnectionCmd::Stop) = cmd_rx.recv() => {
                    mgr.shutdown().await;
                    if let Some(mut failed) = parked.take() {
                        failed.shutdown().await;
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
                    if matches!(cmd_rx.try_recv(), Ok(ConnectionCmd::Stop)) {
                        mgr.shutdown().await;
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
                        mgr.shutdown().await;
                        if let Some(mut failed) = parked.take() {
                            failed.shutdown().await;
                        }
                        if grant_fixable(&e) {
                            sender.emit(AppMsg::TunGrantRequired(generation));
                        }
                        report(ProcessState::Error(e.to_string()), None);
                        return;
                    }
                    failures.push(CandidateFailure::new(
                        &candidate_label,
                        &e.to_string(),
                        &candidate_address,
                        candidate_port,
                    ));
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
            let mut log_rx = mgr.subscribe_logs();
            let log_forwarder = tokio::spawn(async move {
                loop {
                    match log_rx.recv().await {
                        Ok(ProcessEvent::LogLine(line)) => {
                            log_sender.emit(AppMsg::ProcessLogLine(generation, line.content));
                        }
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            loop {
                tokio::select! {
                    Some(ConnectionCmd::Stop) = cmd_rx.recv() => {
                        mgr.shutdown().await;
                        halt(state_forwarder).await;
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
                                failures.push(CandidateFailure::new(
                                    &candidate_label,
                                    &msg,
                                    &candidate_address,
                                    candidate_port,
                                ));
                                break;
                            }
                            _ => {
                                halt(state_forwarder).await;
                                report(ProcessState::Stopped, None);
                                return;
                            }
                        }
                    }
                }
            }
            halt(state_forwarder).await;
            halt(log_forwarder).await;
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

/// Stops a forwarder and waits until it can no longer emit, so nothing it
/// relays can land after the terminal state that follows.
async fn halt(task: JoinHandle<()>) {
    task.abort();
    let _ = task.await;
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

/// Replaces the candidate's own port where it stands alone. The boundary check
/// keeps `443` from being clipped out of `14430` or `10.4.43.1`.
fn mask_port(text: &str, port: u16) -> String {
    let needle = port.to_string();
    let bytes = text.as_bytes();
    let free = |c: Option<u8>| !c.is_some_and(|c| c.is_ascii_digit() || c == b'.');
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with(&needle)
            && free(i.checked_sub(1).map(|j| bytes[j]))
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
        }
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
    async fn failover_reports_no_stopped_and_stop_reports_one() {
        let stub = stub(
            r#"[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; grep -q 203.0.113.1 "$3" && exit 1; exec sleep 30"#,
        );
        let (handle, rx) = connect(
            &stub,
            singbox_settings(),
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

        handle.stop();
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
        assert!(msg.contains("203.0.113.1: 3 crashes"), "{msg}");
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
        let (handle, rx) = connect(&stub, singbox_settings(), vec![candidate("203.0.113.1")]);

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

        handle.stop();
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
        let (handle, rx) = connect(&stub, singbox_settings(), vec![candidate("203.0.113.1")]);

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

        handle.stop();
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
