use std::path::PathBuf;

use std::sync::Arc;

use tokio::sync::{Mutex, broadcast, mpsc};
use v2ray_rs_core::config::ConfigWriter;
use v2ray_rs_core::models::{
    AppSettings, BackendType, ConnectionMetadata, ConnectionNodeRef, DnsHijackMode, HostOverride,
    ManualNode, ProxyNode, RoutingRule, Subscription, resolve_effective_config,
};
use v2ray_rs_core::resolve::{ConnectionCandidate, resolve_via_nodes};
use v2ray_rs_process::{ProcessEvent, ProcessState, TunRuntime};

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

        // A Stop that arrived while we were queued behind the previous
        // connection's teardown must not be answered by starting anyway.
        if matches!(cmd_rx.try_recv(), Ok(ConnectionCmd::Stop)) {
            sender.emit(AppMsg::ProcessStateConnection(
                generation,
                ProcessState::Stopped,
                None,
            ));
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

        for candidate in candidates {
            if matches!(cmd_rx.try_recv(), Ok(ConnectionCmd::Stop)) {
                sender.emit(AppMsg::ProcessStateConnection(
                    generation,
                    ProcessState::Stopped,
                    None,
                ));
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
            let pinned = pin_node_addresses(&mut effective_settings, &nodes).await;
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

            let mut mgr = v2ray_rs_process::ProcessManager::new(
                binary_path.clone(),
                config_path,
                pid_path.clone(),
                Some(geodata_dir.clone()),
            )
            .with_tun(build_tun_runtime(&settings, pinned))
            .with_backend(settings.backend.backend_type);

            match mgr.start_with_connection(Some(meta.clone())).await {
                Ok(()) => {
                    // A Disconnect clicked while the start was in flight sits
                    // queued until here; honor it instead of flashing the UI
                    // back to Connected with a dead handle.
                    if matches!(cmd_rx.try_recv(), Ok(ConnectionCmd::Stop)) {
                        mgr.shutdown().await;
                        sender.emit(AppMsg::ProcessStateConnection(
                            generation,
                            ProcessState::Stopped,
                            None,
                        ));
                        return;
                    }
                    sender.emit(AppMsg::ProcessStateConnection(
                        generation,
                        ProcessState::Running,
                        Some(meta.clone()),
                    ));
                }
                Err(e) => {
                    failures.push(format!("{candidate_label}: {e}"));
                    mgr.shutdown().await;
                    continue;
                }
            }

            let state_sender = sender.clone();
            let mut state_rx = mgr.subscribe();
            tokio::spawn(async move {
                loop {
                    match state_rx.recv().await {
                        Ok(ProcessEvent::StateChanged { to, connection, .. }) => {
                            // Terminal errors stay with the supervising loop,
                            // which may fail over to the next candidate; only
                            // it decides what the app finally sees.
                            if !matches!(to, ProcessState::Error(_)) {
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
            tokio::spawn(async move {
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

            let mut stop_requested = false;
            loop {
                tokio::select! {
                    Some(cmd) = cmd_rx.recv() => {
                        match cmd {
                            ConnectionCmd::Stop => {
                                mgr.shutdown().await;
                                return;
                            }
                        }
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
                                stop_requested = true;
                                break;
                            }
                        }
                    }
                }
            }
            if stop_requested {
                return;
            }
            mgr.shutdown().await;
        }

        let msg = summarize_failures(&failures);
        sender.emit(AppMsg::ProcessStateConnection(
            generation,
            ProcessState::Error(msg),
            None,
        ));
    });

    ConnectionHandle { cmd_tx }
}

/// Builds the TUN runtime from settings, or `None` when TUN is off or the
/// backend is v2ray (which has no native TUN inbound).
const PIN_LOOKUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Pins every hostname-addressed node to concrete IPs in `dns.hosts`.
///
/// Returns false if any of them could not be resolved. The caller uses that to
/// leave DNS capture off for this connect: capturing port 53 while the backend
/// still needs the OS resolver to find its own server would send that lookup
/// into the tunnel it is trying to build.
async fn pin_node_addresses(settings: &mut AppSettings, nodes: &[ProxyNode]) -> bool {
    let mut all_pinned = true;

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
            Err(_) => {
                log::warn!("cannot pin {host}: lookup timed out");
                all_pinned = false;
            }
            Ok(Ok(addrs)) => {
                let mut found = false;
                for addr in addrs {
                    settings.dns.hosts.push(HostOverride {
                        domain: host.to_string(),
                        ip: addr.ip().to_string(),
                    });
                    found = true;
                }
                if !found {
                    all_pinned = false;
                }
            }
            Ok(Err(err)) => {
                log::warn!("cannot pin {host}: {err}");
                all_pinned = false;
            }
        }
    }

    all_pinned
}

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
        // `nodes_pinned` is the safety interlock: an unpinned node still needs
        // the OS resolver to be found, and capturing port 53 would swallow that
        // lookup.
        capture_dns: backend == BackendType::Xray
            && settings.tun.dns_hijack == DnsHijackMode::Hijack
            && nodes_pinned,
    })
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
