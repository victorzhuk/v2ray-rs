use std::path::PathBuf;

use std::sync::Arc;

use tokio::sync::{Mutex, broadcast, mpsc};
use tokio::task::JoinHandle;
use v2ray_rs_core::config::ConfigWriter;
use v2ray_rs_core::models::{
    AppSettings, BackendType, ConnectionMetadata, ConnectionNodeRef, DnsHijackMode, HostOverride,
    ManualNode, ProxyNode, RoutingRule, Subscription, resolve_effective_config,
};
use v2ray_rs_core::resolve::{ConnectionCandidate, resolve_via_nodes};
use v2ray_rs_process::{ProcessEvent, ProcessManager, ProcessState, TunRuntime};

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
    let ConnectionRequest {
        binary_path,
        candidates,
        writer,
        pid_path,
        geodata_dir,
        settings,
        enabled_rules,
        subscriptions,
        manual_nodes,
        lifecycle,
        generation,
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

        for candidate in candidates {
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
            let pinned = hosts_cover_nodes(&effective_settings, &nodes);
            let config_path =
                match writer.write_config(&nodes, &effective_rules, &effective_settings) {
                    Ok(path) => path,
                    Err(e) => {
                        failures.push(format!("{candidate_label}: config generation failed: {e}"));
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
            let mut mgr = ProcessManager::new(
                binary_path.clone(),
                config_path,
                pid_path.clone(),
                Some(geodata_dir.clone()),
            )
            .with_tun(tun)
            .with_backend(settings.backend.backend_type);

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
                    failures.push(format!("{candidate_label}: {e}"));
                    parked = Some(mgr);
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
                            log_sender.emit(AppMsg::ProcessLogLine(line.content));
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
                                failures.push(format!("{candidate_label}: {msg}"));
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
        }

        report(ProcessState::Error(summarize_failures(&failures)), None);
    });

    ConnectionHandle { cmd_tx }
}

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

fn relays(state: &ProcessState) -> bool {
    matches!(
        state,
        ProcessState::Starting | ProcessState::Running | ProcessState::Stopping
    )
}

/// Stops a forwarder and waits until it can no longer emit, so nothing it
/// relays can land after the terminal state that follows.
async fn halt(task: JoinHandle<()>) {
    task.abort();
    let _ = task.await;
}

fn summarize_failures(failures: &[String]) -> String {
    if failures.is_empty() {
        return "All candidates failed".into();
    }

    let preview = failures
        .iter()
        .take(3)
        .map(String::as_str)
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
    use v2ray_rs_core::models::{DnsStrategy, ShadowsocksConfig, TunConfig};
    use v2ray_rs_core::persistence::AppPaths;
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
        let (tx, rx) = relm4::channel::<AppMsg>();
        let handle = spawn(
            ConnectionRequest {
                binary_path: stub.binary.clone(),
                candidates,
                writer: ConfigWriter::new(&settings, &stub.paths),
                pid_path: stub.paths.pid_file_path(),
                geodata_dir: stub.paths.geodata_dir(),
                settings,
                enabled_rules: Vec::new(),
                subscriptions: Vec::new(),
                manual_nodes: Vec::new(),
                lifecycle: TunLifecycle::default(),
                generation: GENERATION,
            },
            tx,
        );
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

    #[tokio::test(flavor = "multi_thread")]
    async fn failover_reports_no_stopped_and_stop_reports_one() {
        let stub = stub(
            r#"[ "$1" = check ] && exit 0; grep -q 203.0.113.1 "$3" && exit 1; exec sleep 30"#,
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
            r#"[ "$1" = check ] && { grep -q 203.0.113.3 "$3" && exit 1; exit 0; }; grep -q 203.0.113.1 "$3" && exit 1; exec sleep 30"#,
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
