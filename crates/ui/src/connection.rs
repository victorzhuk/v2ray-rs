use std::path::PathBuf;

use tokio::sync::{broadcast, mpsc};
use v2ray_rs_core::config::ConfigWriter;
use v2ray_rs_core::models::{
    AppSettings, BackendType, ConnectionMetadata, ConnectionNodeRef, RoutingRule, Subscription,
    resolve_effective_config,
};
use v2ray_rs_core::resolve::ConnectionCandidate;
use v2ray_rs_process::{ProcessEvent, ProcessState, TunRuntime};

use crate::app::AppMsg;

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
    } = request;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<ConnectionCmd>(4);

    tokio::spawn(async move {
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
                sender.emit(AppMsg::ProcessStateConnection(ProcessState::Stopped, None));
                return;
            }
            let candidate_label = candidate
                .node
                .remark()
                .unwrap_or(candidate.node.address())
                .to_string();
            let (effective_rules, effective_settings) = resolve_effective_config(
                &candidate.node_ref,
                &subscriptions,
                &enabled_rules,
                &settings,
            );
            let config_path = match writer.write_config(
                std::slice::from_ref(&candidate.node),
                &effective_rules,
                &effective_settings,
            ) {
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
            .with_tun(build_tun_runtime(&settings))
            .with_backend(settings.backend.backend_type);

            match mgr.start_with_connection(Some(meta.clone())).await {
                Ok(()) => {
                    // A Disconnect clicked while the start was in flight sits
                    // queued until here; honor it instead of flashing the UI
                    // back to Connected with a dead handle.
                    if matches!(cmd_rx.try_recv(), Ok(ConnectionCmd::Stop)) {
                        mgr.shutdown().await;
                        sender.emit(AppMsg::ProcessStateConnection(ProcessState::Stopped, None));
                        return;
                    }
                    sender.emit(AppMsg::ProcessStateConnection(
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
                                state_sender.emit(AppMsg::ProcessStateConnection(to, connection));
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
            ProcessState::Error(msg),
            None,
        ));
    });

    ConnectionHandle { cmd_tx }
}

/// Builds the TUN runtime from settings, or `None` when TUN is off or the
/// backend is v2ray (which has no native TUN inbound).
fn build_tun_runtime(settings: &AppSettings) -> Option<TunRuntime> {
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
