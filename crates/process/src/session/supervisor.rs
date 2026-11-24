use super::builders::*;
use super::cleanup::*;
use super::*;

pub fn spawn(request: SessionRequest, sender: EventSender) -> SessionHandle {
    spawn_with(request, sender, |mgr| mgr, crate::connected_subnets)
}

/// `spawn` with a caller-supplied hook over every candidate's manager and a
/// connected-subnet collector. Exposed only to tests so a joined UI test can
/// pin host probes and helper paths without changing the production chain.
#[cfg(any(test, feature = "test-utils"))]
pub fn spawn_configured(
    request: SessionRequest,
    sender: EventSender,
    configure: impl Fn(ProcessManager) -> ProcessManager + Send + 'static,
    collect: impl FnOnce(&str) -> std::io::Result<Vec<ipnet::IpNet>> + Send + 'static,
) -> SessionHandle {
    spawn_with(request, sender, configure, collect)
}

/// `spawn` with a hook over every candidate's manager, applied after the
/// production builder chain, and a connected-subnet collector so tests can
/// inject a deterministic snapshot.
pub(super) fn spawn_with(
    request: SessionRequest,
    sender: EventSender,
    configure: impl Fn(ProcessManager) -> ProcessManager + Send + 'static,
    collect: impl FnOnce(&str) -> std::io::Result<Vec<ipnet::IpNet>> + Send + 'static,
) -> SessionHandle {
    let SessionRequest {
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
        last_good_pins,
    } = request;
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<SessionCmd>();

    tokio::spawn(async move {
        // Held until this task returns, by every exit path including the early
        // `return`s below, so the next connect cannot start setting up while
        // this one is still tearing down.
        let _lifecycle = lifecycle.lock().await;
        let report = |state: ProcessState, connection: Option<ConnectionMetadata>| {
            sender.emit(SessionEvent::State(generation, state, connection));
        };
        // Managers whose cleanup failed before a retry was possible (for
        // example a superseded candidate parked while the next one runs):
        // they still own kernel state, so they stay here until a retry
        // succeeds instead of being dropped with rules installed.
        let mut orphaned: Vec<ProcessManager> = Vec::new();

        // A Stop that arrived while we were queued behind the previous
        // connection's teardown must not be answered by starting anyway.
        if matches!(cmd_rx.try_recv(), Ok(SessionCmd::Stop(_))) {
            report(ProcessState::Stopped, None);
            return;
        }

        // Reap any orphaned backend from a previous run before spawning ours.
        // Done here, off the GTK thread, so the Connect click never blocks the
        // UI while an orphan is signalled and waited on.
        {
            let orphan_pid = pid_path.clone();
            let cleanup = tokio::task::spawn_blocking(move || {
                crate::PidFile::new(orphan_pid).check_and_kill_orphaned()
            })
            .await;
            // A failed orphan reap must not start a session that could race
            // the survivor for ports or the TUN interface.
            let reaped = match cleanup {
                Ok(Ok(reaped)) => reaped,
                Ok(Err(err)) => {
                    report(
                        ProcessState::Error(format!("Orphan cleanup failed: {err}")),
                        None,
                    );
                    return;
                }
                Err(err) => {
                    report(
                        ProcessState::Error(format!("Orphan cleanup failed: {err}")),
                        None,
                    );
                    return;
                }
            };
            if reaped {
                log::info!("reaped orphaned backend from a previous run");
            }
        }

        // Collected once per connection, before any candidate: the snapshot
        // feeds every config write, including failovers, so a Docker bridge is
        // excluded from TUN capture for the whole session. A host discovery
        // failure is terminal — no candidate can produce a correct TUN config
        // without it.
        let connected_subnets: Vec<ipnet::IpNet> =
            if settings.backend.backend_type == BackendType::SingBox && settings.tun.enabled {
                let iface = settings.tun.interface_name.clone();
                let collected = tokio::task::spawn_blocking(move || collect(&iface));
                tokio::pin!(collected);
                tokio::select! {
                    biased;
                    Some(SessionCmd::Stop(_)) = cmd_rx.recv() => {
                        report(ProcessState::Stopped, None);
                        return;
                    }
                    joined = &mut collected => match joined {
                        Ok(Ok(subnets)) => subnets,
                        Ok(Err(err)) => {
                            report(
                                ProcessState::Error(format!("Collect connected subnets: {err}")),
                                None,
                            );
                            return;
                        }
                        Err(err) => {
                            report(
                                ProcessState::Error(format!("Collect connected subnets: {err}")),
                                None,
                            );
                            return;
                        }
                    },
                }
            } else {
                Vec::new()
            };

        let host = tokio::task::spawn_blocking(|| HostTools {
            bypass_uid: nix::unistd::User::from_name(crate::BYPASS_USER)
                .ok()
                .flatten()
                .map(|u| u.uid.as_raw()),
            helper_path: crate::helper_path(),
            antidpi_path: crate::antidpi_path(),
        })
        .await;
        let host = match host {
            Ok(host) => host,
            Err(join) => {
                report(
                    ProcessState::Error(format!("Resolve host tools: {join}")),
                    None,
                );
                return;
            }
        };

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
            sender.emit(SessionEvent::LogLines(
                generation,
                vec![STRICT_ROUTE_NOTICE.into()],
            ));
        }

        // Same reasoning as the strict-route notice: outside the candidate loop
        // so a failover cannot repeat it.
        if let Some(warning) = v2ray_tun_warning(&settings) {
            if let Some(log) = &backend_log {
                log.append_line("warning", &warning);
            }
            sender.emit(SessionEvent::LogLines(generation, vec![warning]));
        }

        // Strict xray TUN installs kill-switch routes before the first start,
        // so only its first few planned candidates can be looked up live. The
        // rest fall back to last-good pins or IP literals.
        let strict_lookahead = settings.tun.enabled
            && settings.tun.strict_route
            && !drop_strict_route
            && settings.backend.backend_type == BackendType::Xray;
        let mut resolved: HashMap<String, Vec<IpAddr>> = HashMap::new();
        if strict_lookahead {
            let planned = candidates.len().min(TUN_PIN_LOOKAHEAD);
            let hosts = pin_hosts(
                &candidates[..planned],
                &subscriptions,
                &manual_nodes,
                &enabled_rules,
                &settings,
            );
            let lookups = resolve_pins(hosts, &last_good_pins, PIN_LOOKUP_TIMEOUT, os_lookup);
            tokio::pin!(lookups);
            resolved = tokio::select! {
                biased;
                Some(SessionCmd::Stop(_)) = cmd_rx.recv() => {
                    report(ProcessState::Stopped, None);
                    return;
                }
                resolved = &mut lookups => resolved,
            };
        }

        // Once per connection, like the strict-route notice, so a failover
        // cannot repeat it.
        let mut capture_notice_sent = false;
        // Same once-per-connection lifetime: a failover candidate must not
        // repeat the fallback-DNS notice.
        let mut dns_notice_sent = false;
        let warnings = Arc::new(std::sync::Mutex::new(BackendWarnings::new(
            settings.backend.backend_type,
        )));
        // Subscriptions whose REALITY auth rejection was already offered a
        // refresh in this connection: failover across several nodes of the same
        // subscription must not stack repeated offers.
        let mut refresh_offered: HashSet<uuid::Uuid> = HashSet::new();
        let total = candidates.len();
        'candidates: for (index, candidate) in candidates.into_iter().enumerate() {
            let position = index + 1;
            if let Ok(SessionCmd::Stop(reason)) = cmd_rx.try_recv() {
                let mut pending = Vec::new();
                if let Some(failed) = parked.take() {
                    stop_or_park(failed, reason, &mut pending).await;
                }
                finish_or_retry_cleanup(
                    pending,
                    &mut orphaned,
                    generation,
                    &mut cmd_rx,
                    &sender,
                    ProcessState::Stopped,
                )
                .await;
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
            let mut applied = Vec::new();
            if pins_enabled(&effective_settings) {
                let hosts = pin_hosts_for(&nodes, &effective_settings);
                let missing: Vec<(String, u16)> = hosts
                    .iter()
                    .filter(|(host, _)| !resolved.contains_key(host))
                    .cloned()
                    .collect();

                if !missing.is_empty() {
                    // Strict xray routes and parked kill switches both make a
                    // live lookup a self-capture; use the last good addresses.
                    if strict_lookahead || !probe_allowed(parked.as_ref()) {
                        resolved.extend(last_good_resolution(&missing, &last_good_pins));
                    } else {
                        let lookups =
                            resolve_pins(missing, &last_good_pins, PIN_LOOKUP_TIMEOUT, os_lookup);
                        tokio::pin!(lookups);
                        let found = tokio::select! {
                            biased;
                            Some(SessionCmd::Stop(reason)) = cmd_rx.recv() => {
                                let mut pending = Vec::new();
                                if let Some(failed) = parked.take() {
                                    stop_or_park(failed, reason, &mut pending).await;
                                }
                                finish_or_retry_cleanup(pending, &mut orphaned, generation, &mut cmd_rx, &sender, ProcessState::Stopped).await;
                                return;
                            }
                            found = &mut lookups => found,
                        };
                        resolved.extend(found);
                    }
                }

                let pins: HashMap<String, Vec<IpAddr>> = hosts
                    .iter()
                    .filter_map(|(host, _)| {
                        resolved
                            .get(host)
                            .map(|addrs| (host.clone(), addrs.clone()))
                    })
                    .collect();
                // A parked candidate's tunnel is still up, so a probe would
                // measure its kill switch rather than the node.
                let pins = if probe_allowed(parked.as_ref()) && !pins.is_empty() {
                    let probes = probe_pins(&nodes, &pins);
                    tokio::pin!(probes);
                    tokio::select! {
                        biased;
                        Some(SessionCmd::Stop(reason)) = cmd_rx.recv() => {
                            let mut pending = Vec::new();
                            if let Some(failed) = parked.take() {
                                stop_or_park(failed, reason, &mut pending).await;
                            }
                            finish_or_retry_cleanup(pending, &mut orphaned, generation, &mut cmd_rx, &sender, ProcessState::Stopped).await;
                            return;
                        }
                        pins = &mut probes => pins,
                    }
                } else {
                    pins
                };
                applied = apply_pins(&mut effective_settings, &nodes, &pins);
            }
            if drop_strict_route {
                effective_settings.tun.strict_route = false;
            }
            let unpinned = unpinned_host(&effective_settings, &nodes);
            let pinned = unpinned.is_none();
            let mut tun = build_tun_runtime(&effective_settings, pinned, &host);
            // A parked tunnel belongs to the previous candidate's TUN session.
            // When the next candidate runs without TUN, that session would
            // keep capturing traffic for a node the user has left.
            if tun.is_none()
                && parked.as_ref().is_some_and(ProcessManager::has_tun_runtime)
                && let Some(stale) = parked.take()
            {
                stop_or_park(stale, StopReason::NodeSwitch, &mut orphaned).await;
            }
            // Shape-checked before any kernel touch or config write, mirroring
            // the config-generation failure handling: a hand-edited settings
            // file fails this candidate like any other per-candidate failure.
            let antidpi_runtime =
                match build_antidpi_runtime(&effective_settings, &host.antidpi_path) {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        record_failure(
                            &mut failures,
                            CandidateFailure::new(
                                &candidate_label,
                                &format!("anti-DPI settings invalid: {err}"),
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
            let config_path = ResolvedNodeSet::from_nodes(&nodes).and_then(|nodes| {
                writer.write_config(
                    &nodes,
                    &effective_rules,
                    &effective_settings,
                    &connected_subnets,
                )
            });
            let config_path = match config_path {
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
            // Written before any kernel touch: startup and terminal-error
            // recovery are gated on this marker, so a session must not start
            // if the recovery pass cannot be recorded.
            if let Some(rt) = &mut tun {
                let session_paths = paths.clone();
                let (backend, iface) = (rt.backend, rt.iface.clone());
                let prepared = tokio::task::spawn_blocking(move || {
                    prepare_tun_session(&session_paths, backend, &iface)
                })
                .await
                .unwrap_or_else(|join| Err(std::io::Error::other(join).into()));
                let session = match prepared {
                    Ok(session) => session,
                    Err(err) => {
                        // No candidate can start without a record of its TUN session,
                        // so what earlier candidates left armed is released here
                        // instead of being left for startup recovery.
                        let mut pending = Vec::new();
                        if let Some(stale) = parked.take() {
                            stop_or_park(stale, StopReason::StartFailed, &mut pending).await;
                        }
                        finish_or_retry_cleanup(
                            pending,
                            &mut orphaned,
                            generation,
                            &mut cmd_rx,
                            &sender,
                            ProcessState::Error(format!("Failed to record TUN session: {err}")),
                        )
                        .await;
                        return;
                    }
                };
                rt.journal = Some(crate::TunJournal {
                    path: paths.tun_session_path(),
                    session_id: session
                        .ownership
                        .as_ref()
                        .expect("prepared ownership")
                        .session_id
                        .to_string(),
                });
            }

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

            let node_hosts: Vec<&str> = nodes.iter().map(|node| node.address()).collect();
            let mut summary = effective_dns(
                settings.backend.backend_type,
                &effective_settings,
                &effective_rules,
                &node_hosts,
            );
            if uses_imported_profile(&candidate.node_ref, &subscriptions) {
                summary.mark_profile();
            }
            if !dns_notice_sent
                && let Some(notice) = fallback_dns_notice(
                    &summary,
                    effective_settings.dns.enabled,
                    tun.as_ref().is_some(),
                    settings.backend.backend_type,
                )
            {
                dns_notice_sent = true;
                if let Some(log) = &backend_log {
                    log.append_line("notice", notice);
                }
                sender.emit(SessionEvent::LogLines(generation, vec![notice.to_string()]));
            }
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
            if !capture_notice_sent
                && tun
                    .as_ref()
                    .is_some_and(|rt| rt.backend == BackendType::Xray)
                && effective_settings.tun.dns_hijack == DnsHijackMode::Hijack
                && let Some(host) = unpinned
            {
                capture_notice_sent = true;
                let notice = dns_capture_off_notice(host);
                if let Some(log) = &backend_log {
                    log.append_line("notice", &notice);
                }
                sender.emit(SessionEvent::LogLines(generation, vec![notice]));
                sender.emit(SessionEvent::Toast(dns_capture_off_toast(host)));
            }
            let mut mgr = configure(
                ProcessManager::new(
                    binary_path.clone(),
                    config_path,
                    pid_path.clone(),
                    Some(geodata_dir.clone()),
                )
                .with_tun(tun)
                .with_antidpi(antidpi_runtime)
                .with_backend(settings.backend.backend_type)
                .with_log_file(backend_log.clone())
                .with_session_fields(session_fields)
                .with_session_extra(summary.log_lines())
                .with_ready_probe(effective_settings.local_endpoint(effective_settings.socks_port)),
            );
            // xray prints deprecation warnings milliseconds after spawn, before
            // start returns, so a later subscription would miss them.
            let mut log_rx = mgr.subscribe_logs();
            // Subscribed before the start for the same reason: the engine's
            // post-ready exit is reported on this lossless channel, never on
            // the log broadcast.
            let mut advisory_rx = mgr.subscribe_advisories();

            let started = tokio::select! {
                biased;
                Some(SessionCmd::Stop(reason)) = cmd_rx.recv() => {
                    let mut pending = Vec::new();
                    stop_or_park(mgr, reason, &mut pending).await;
                    if let Some(failed) = parked.take() {
                        stop_or_park(failed, reason, &mut pending).await;
                    }
                    finish_or_retry_cleanup(pending, &mut orphaned, generation, &mut cmd_rx, &sender, ProcessState::Stopped).await;
                    return;
                }
                started = mgr.start_with_connection(Some(meta.clone())) => started,
            };

            match started {
                Ok(()) => {
                    supersede_parked(&mut parked, &mgr, &mut orphaned).await;
                    // A Disconnect clicked while the start was in flight sits
                    // queued until here; honor it instead of flashing the UI
                    // back to Connected with a dead handle.
                    if let Ok(SessionCmd::Stop(reason)) = cmd_rx.try_recv() {
                        let mut pending = Vec::new();
                        stop_or_park(mgr, reason, &mut pending).await;
                        finish_or_retry_cleanup(
                            pending,
                            &mut orphaned,
                            generation,
                            &mut cmd_rx,
                            &sender,
                            ProcessState::Stopped,
                        )
                        .await;
                        return;
                    }
                    report(ProcessState::Running, Some(meta.clone()));
                    if !applied.is_empty() {
                        sender.emit(SessionEvent::NodePins(generation, applied));
                    }
                }
                Err(e) => {
                    if e.is_host_level() {
                        // A host-level failure blocks every candidate, so
                        // failover is over: tear down what is parked and
                        // report the host error itself, not a summary.
                        let mut pending = Vec::new();
                        stop_or_park(mgr, StopReason::StartFailed, &mut pending).await;
                        if let Some(failed) = parked.take() {
                            stop_or_park(failed, StopReason::StartFailed, &mut pending).await;
                        }
                        if grant_fixable(&e) {
                            sender.emit(SessionEvent::TunGrantRequired(generation));
                        }
                        finish_or_retry_cleanup(
                            pending,
                            &mut orphaned,
                            generation,
                            &mut cmd_rx,
                            &sender,
                            ProcessState::Error(e.to_string()),
                        )
                        .await;
                        return;
                    }
                    // A start the backend itself failed (config rejected, exit
                    // before ready) has its output in the manager's buffer, so
                    // a known signature classifies here exactly as it does for
                    // a later crash give-up.
                    let classified = classify_candidate(&mgr, settings.backend.backend_type);
                    let reason = classified
                        .as_ref()
                        .map(|c| c.cause.to_string())
                        .unwrap_or_else(|| e.to_string());
                    record_failure(
                        &mut failures,
                        CandidateFailure::new(
                            &candidate_label,
                            &reason,
                            &candidate_address,
                            candidate_port,
                        ),
                        position,
                        total,
                    );
                    if let (
                        Some(classification),
                        ConnectionNodeRef::Subscription {
                            subscription_id, ..
                        },
                    ) = (classified, candidate.node_ref)
                    {
                        emit_refresh_offer(
                            &sender,
                            &subscriptions,
                            &mut refresh_offered,
                            subscription_id,
                            &candidate_label,
                            classification.cause,
                        );
                    }
                    // The candidate is parked with a dead backend: its engine
                    // and rules protect nothing and would race the next
                    // candidate's queue binding, so they end here. A cleanup
                    // that did not finish leaves the rules installed, so the
                    // failover ends with that host error instead of racing
                    // every later candidate. The parked candidate keeps its
                    // TUN kill-switch until a replacement adopts that
                    // identity or a teardown releases it.
                    let mut pending = Vec::new();
                    if mgr.teardown_antidpi().await.is_err() {
                        pending.push(mgr);
                        if let Some(stale) = parked.take() {
                            stop_or_park(stale, StopReason::StartFailed, &mut pending).await;
                        }
                        finish_or_retry_cleanup(
                            pending,
                            &mut orphaned,
                            generation,
                            &mut cmd_rx,
                            &sender,
                            ProcessState::Error(terminal_failure(&failures)),
                        )
                        .await;
                        return;
                    }
                    supersede_parked(&mut parked, &mgr, &mut orphaned).await;
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
                                state_sender.emit(SessionEvent::State(generation, to, connection));
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
            let warnings = Arc::clone(&warnings);
            let log_forwarder = tokio::spawn(async move {
                // Only xray's log lines are known to mark DNS failures.
                let mut dns = (backend == BackendType::Xray).then(DnsFailureWindow::default);
                let mut dns_failing = false;
                let mut degraded_seen = false;
                let mut recheck = tokio::time::interval(health_timing.dns_recheck);
                let mut batcher = LogBatcher::default();
                let mut flush = tokio::time::interval(LOG_FLUSH_INTERVAL);
                flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tokio::select! {
                        event = log_rx.recv() => match event {
                            Ok(ProcessEvent::LogLine(line)) => {
                                if let Some(window) = &mut dns {
                                    window.observe(&line.content, Instant::now());
                                }
                                let toast = warnings
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .toast_for(&line.content);
                                batcher.push(line.content);
                                if batcher.is_full() {
                                    flush_log_batch(&log_sender, generation, batcher.take());
                                }
                                if let Some(toast) = toast {
                                    log_sender.emit(SessionEvent::Toast(toast));
                                }
                            }
                            Ok(_) => continue,
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        },
                        advisory = advisory_rx.recv() => match advisory {
                            Some(ProcessAdvisory::AntidpiDegraded { detail }) => {
                                // One notice per generation: engine activity
                                // later in the same connection must not spam
                                // the UI, and the proxy state stays untouched.
                                if !degraded_seen {
                                    degraded_seen = true;
                                    log_sender.emit(SessionEvent::EngineDegraded {
                                        generation,
                                        detail,
                                    });
                                }
                            }
                            // The manager dropped its advisory sender: nothing
                            // left can degrade this session, so finish the
                            // forwarder cleanly instead of polling forever.
                            None => {
                                flush_log_batch(&log_sender, generation, batcher.take());
                                break;
                            }
                        },
                        _ = recheck.tick(), if dns.is_some() => {}
                        _ = flush.tick() => {
                            flush_log_batch(&log_sender, generation, batcher.take());
                        }
                    }
                    if let Some(window) = &dns
                        && let Some(failing) = dns_flip(window, &mut dns_failing, Instant::now())
                    {
                        log_sender.emit(SessionEvent::DnsHealth(generation, failing));
                    }
                }
                flush_log_batch(&log_sender, generation, batcher.take());
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
                    Some(SessionCmd::Stop(reason)) = cmd_rx.recv() => {
                        let mut pending = Vec::new();
                        stop_or_park(mgr, reason, &mut pending).await;
                        halt(forwarders).await;
                        finish_or_retry_cleanup(pending, &mut orphaned, generation, &mut cmd_rx, &sender, ProcessState::Stopped).await;
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
                                let classified =
                                    classify_candidate(&mgr, settings.backend.backend_type);
                                let reason = classified
                                    .as_ref()
                                    .map(|c| c.cause.to_string())
                                    .unwrap_or(msg);
                                record_failure(
                                    &mut failures,
                                    CandidateFailure::new(
                                        &candidate_label,
                                        &reason,
                                        &candidate_address,
                                        candidate_port,
                                    ),
                                    position,
                                    total,
                                );
                                if let (Some(classification), ConnectionNodeRef::Subscription {
                                    subscription_id,
                                    ..
                                }) = (classified, candidate.node_ref)
                                {
                                    emit_refresh_offer(
                                        &sender,
                                        &subscriptions,
                                        &mut refresh_offered,
                                        subscription_id,
                                        &candidate_label,
                                        classification.cause,
                                    );
                                }
                                break;
                            }
                            _ => {
                                halt(forwarders).await;
                                let mut pending = Vec::new();
                                stop_or_park(mgr, StopReason::UserStop, &mut pending).await;
                                if let Some(failed) = parked.take() {
                                    stop_or_park(failed, StopReason::UserStop, &mut pending).await;
                                }
                                finish_or_retry_cleanup(pending, &mut orphaned, generation, &mut cmd_rx, &sender, ProcessState::Stopped).await;
                                return;
                            }
                        }
                    }
                }
            }
            halt(forwarders).await;
            // Same as the start-error park above: a crash give-up leaves the
            // backend dead, so the engine and its rules are released here. A
            // cleanup that did not finish leaves the rules installed, so the
            // failover ends with that host error instead of racing every
            // later candidate. The parked candidate keeps its TUN kill-switch
            // until a replacement adopts that identity or a teardown releases
            // it.
            let mut pending = Vec::new();
            if mgr.teardown_antidpi().await.is_err() {
                pending.push(mgr);
                if let Some(stale) = parked.take() {
                    stop_or_park(stale, StopReason::StartFailed, &mut pending).await;
                }
                finish_or_retry_cleanup(
                    pending,
                    &mut orphaned,
                    generation,
                    &mut cmd_rx,
                    &sender,
                    ProcessState::Error(terminal_failure(&failures)),
                )
                .await;
                return;
            }
            supersede_parked(&mut parked, &mgr, &mut orphaned).await;
            parked = Some(mgr);
            if repeats_previous(&failures) {
                break 'candidates;
            }
        }

        // The give-up terminal path drops the parked manager without a stop:
        // release its anti-DPI engine and rules here (the TUN state stays
        // armed on purpose — fail-closed routes must survive until startup
        // recovery — so only the engine and the shared chain are released).
        let mut pending = Vec::new();
        if let Some(mut stale) = parked.take()
            && stale.teardown_antidpi().await.is_err()
        {
            pending.push(stale);
        }
        finish_or_retry_cleanup(
            pending,
            &mut orphaned,
            generation,
            &mut cmd_rx,
            &sender,
            ProcessState::Error(terminal_failure(&failures)),
        )
        .await;
    });

    SessionHandle { cmd_tx }
}

pub(super) fn relays(state: &ProcessState) -> bool {
    matches!(
        state,
        ProcessState::Starting | ProcessState::Running | ProcessState::Stopping
    )
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
    sender: EventSender,
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
                    let outcome = probe::probe_via_http_proxy(
                        probe.proxy,
                        &probe.url,
                        probe.timeout,
                    )
                    .await;
                    if let Some(health) = tracker.record(outcome) {
                        sender.emit(SessionEvent::Health(generation, health));
                    }
                }
            }
        }
    })
}

/// Log lines accumulate for at most this long before delivery.
const LOG_FLUSH_INTERVAL: Duration = Duration::from_millis(50);

/// Maximum lines held in one pending batch; the oldest line is dropped
/// beyond this so a fast backend cannot grow delivery memory without limit.
pub(super) const LOG_BATCH_CAP: usize = 512;

/// A flushed batch of backend log lines with the count dropped before it.
pub(super) struct LogBatch {
    pub(super) lines: Vec<String>,
    pub(super) dropped: usize,
}

/// Buffers backend log lines for batched UI delivery. Ordering is FIFO.
#[derive(Default)]
pub(super) struct LogBatcher {
    lines: Vec<String>,
    dropped: usize,
}

impl LogBatcher {
    /// Buffers one line, dropping the oldest and counting it when full.
    pub(super) fn push(&mut self, line: String) {
        if self.lines.len() >= LOG_BATCH_CAP {
            self.lines.remove(0);
            self.dropped += 1;
        }
        self.lines.push(line);
    }

    /// Whether the batch reached its cap and should flush immediately.
    fn is_full(&self) -> bool {
        self.lines.len() >= LOG_BATCH_CAP
    }

    /// Takes the pending batch, or `None` when nothing is buffered.
    pub(super) fn take(&mut self) -> Option<LogBatch> {
        if self.lines.is_empty() {
            return None;
        }
        Some(LogBatch {
            lines: std::mem::take(&mut self.lines),
            dropped: std::mem::take(&mut self.dropped),
        })
    }
}

/// Emits one batched log message, prepending a drop marker when lines were
/// dropped before this batch.
pub(super) fn flush_log_batch(sender: &EventSender, generation: u64, batch: Option<LogBatch>) {
    let Some(batch) = batch else { return };
    let mut lines = batch.lines;
    if batch.dropped > 0 {
        lines.insert(
            0,
            format!(
                "notice: {} backend log line(s) dropped before delivery under load",
                batch.dropped
            ),
        );
    }
    sender.emit(SessionEvent::LogLines(generation, lines));
}

/// The new DNS failing mark when it differs from the last one reported.
pub(super) fn dns_flip(
    window: &DnsFailureWindow,
    reported: &mut bool,
    now: Instant,
) -> Option<bool> {
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

/// Whether a host-level failure is fixed by granting capabilities to the
/// backend, the route helper, and the anti-DPI helper: the repair the grant
/// prompt covers.
pub(super) fn grant_fixable(e: &ProcessError) -> bool {
    matches!(
        e,
        ProcessError::TunCapabilityMissing(_)
            | ProcessError::TunHelperCapabilityMissing(_)
            | ProcessError::TunHelperEnrollmentRequired
            | ProcessError::TunHelperPermissions(_)
            | ProcessError::AntiDpiHelperCapabilityMissing(_)
            | ProcessError::AntiDpiHelperEnrollmentRequired
            | ProcessError::AntiDpiHelperPermissions(_)
    )
}
