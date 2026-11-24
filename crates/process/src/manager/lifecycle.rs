use super::*;

impl ProcessManager {
    pub async fn start(&mut self) -> Result<(), ProcessError> {
        self.start_with_connection(None).await
    }

    /// Terminal tail of a failed preflight: step through `Starting`, land in
    /// `Error` with the error's own text, and hand the error to `return`. The
    /// `Starting` hop keeps the transition history of a preflight rejection
    /// identical to one that fails after launch.
    pub(super) fn fail_with(
        &mut self,
        connection: Option<&ConnectionMetadata>,
        error: ProcessError,
    ) -> Result<(), ProcessError> {
        self.state
            .transition(ProcessState::Starting, connection.cloned())?;
        let _ = self
            .state
            .transition(ProcessState::Error(error.to_string()), None);
        Err(error)
    }
    pub async fn start_with_connection(
        &mut self,
        connection: Option<ConnectionMetadata>,
    ) -> Result<(), ProcessError> {
        if !self.binary_path.exists() {
            self.state
                .transition(ProcessState::Starting, connection.clone())?;
            let error = ProcessError::BinaryNotFound(self.binary_path.clone());
            let _ = self
                .state
                .transition(ProcessState::Error(error.to_string()), None);
            return Err(error);
        }
        if !self.config_path.exists() {
            self.state
                .transition(ProcessState::Starting, connection.clone())?;
            let error = ProcessError::ConfigMissing(self.config_path.clone());
            let _ = self
                .state
                .transition(ProcessState::Error(error.to_string()), None);
            return Err(error);
        }

        if self.backend == Some(BackendType::SingBox) {
            match self.version_triple().await {
                Some(triple) if triple < SINGBOX_MIN_VERSION => {
                    let error = too_old(BackendType::SingBox, triple, SINGBOX_MIN_VERSION);
                    return self.fail_with(connection.as_ref(), error);
                }
                Some(_) => {}
                None => self.push_notice(unreadable_version(BackendType::SingBox)),
            }
        }

        if self.tun.is_some() {
            if self.backend == Some(BackendType::Xray) {
                match self.version_triple().await {
                    Some(triple) if triple < XRAY_TUN_MIN_VERSION => {
                        let error = too_old(BackendType::Xray, triple, XRAY_TUN_MIN_VERSION);
                        return self.fail_with(connection.as_ref(), error);
                    }
                    Some(triple) if triple < XRAY_TUN_PANIC_FIX_VERSION => {
                        self.push_notice(format!(
                            "warning: xray {} can crash the TUN tunnel on quickly-closed connections (Xray-core #6364, fixed in 26.6.27); occasional auto-reconnects are expected until xray is upgraded",
                            format_triple(triple)
                        ));
                    }
                    Some(_) => {}
                    None => self.push_notice(unreadable_version(BackendType::Xray)),
                }
            }
            // On a nosuid mount the getcap probes below would report the
            // backend as unprivileged no matter what was granted.
            if !self.host_pinned()
                && let Some(text) =
                    unsupported_mount(&self.binary_path, crate::privilege::BACKEND_CAPS).await
            {
                return self
                    .fail_with(connection.as_ref(), ProcessError::TunMountUnsupported(text));
            }

            // Only xray drives the privileged route helper; sing-box
            // self-routes. The gates below check the stored runtime's copy —
            // the exact path the launch path will execute — never a fresh
            // resolution.
            let route_helper = if self.backend == Some(BackendType::Xray) {
                self.tun.as_ref().map(|rt| rt.helper_path.clone())
            } else {
                None
            };
            if let Some(installed) = &route_helper {
                if !self.host_pinned()
                    && let Some(text) =
                        unsupported_mount(installed, crate::privilege::HELPER_CAPS).await
                {
                    return self
                        .fail_with(connection.as_ref(), ProcessError::TunMountUnsupported(text));
                }
                #[cfg(any(test, feature = "test-utils"))]
                let pinned = self
                    .host_probe
                    .as_ref()
                    .and_then(|probe| probe.helper_access.clone());
                #[cfg(not(any(test, feature = "test-utils")))]
                let pinned = None;
                let access = match pinned {
                    Some(access) => access,
                    None => {
                        let path = installed.clone();
                        match tokio::task::spawn_blocking(move || crate::tun::helper_access(&path))
                            .await
                        {
                            Ok(access) => access,
                            Err(join) => {
                                return self.fail_with(
                                    connection.as_ref(),
                                    ProcessError::TunHelperAccessProbe {
                                        path: installed.clone(),
                                        detail: join.to_string(),
                                    },
                                );
                            }
                        }
                    }
                };
                if let Some(error) = ProcessError::from_helper_access(installed, access) {
                    return self.fail_with(connection.as_ref(), error);
                }
            }

            // Root holds every capability in the process already, so probing
            // file capabilities there only adds a failure mode.
            let caps_gate = self.host_pinned()
                || crate::privilege::caps_check_needed(nix::unistd::geteuid().as_raw());
            let getcap = self.getcap_program();
            if caps_gate {
                let binary = self.binary_path.clone();
                let program = getcap.clone();
                let probe = tokio::task::spawn_blocking(move || {
                    crate::privilege::probe_net_admin(&program, &binary)
                })
                .await;
                let cap = match probe {
                    Ok(inner) => inner,
                    Err(join) => Err(crate::privilege::PrivilegeError::Probe(
                        self.binary_path.clone(),
                        join.to_string(),
                    )),
                };
                match cap {
                    Ok(true) => {}
                    other => {
                        let error = match other {
                            Ok(false) => {
                                ProcessError::TunCapabilityMissing(self.binary_path.clone())
                            }
                            Err(e) => ProcessError::TunCapabilityProbe(e.to_string()),
                            Ok(true) => unreachable!(),
                        };
                        return self.fail_with(connection.as_ref(), error);
                    }
                }
            }

            if caps_gate && let Some(helper) = route_helper {
                let target = helper.clone();
                let probe = tokio::task::spawn_blocking(move || {
                    crate::privilege::probe_net_admin(&getcap, &target)
                })
                .await;
                let cap = match probe {
                    Ok(inner) => inner,
                    Err(join) => Err(crate::privilege::PrivilegeError::Probe(
                        helper.clone(),
                        join.to_string(),
                    )),
                };
                match cap {
                    Ok(true) => {}
                    other => {
                        let error = match other {
                            Ok(false) => ProcessError::TunHelperCapabilityMissing(helper),
                            Err(e) => ProcessError::TunCapabilityProbe(e.to_string()),
                            Ok(true) => unreachable!(),
                        };
                        return self.fail_with(connection.as_ref(), error);
                    }
                }
            }
        }

        // Anti-DPI helper gates: the same access classification the route
        // helper gets, plus the engine's two-capability requirement. Every
        // failure here is host-level, so failover never retries it, and the
        // grant action can surface from the error text alone.
        if let Some(rt) = self.antidpi.clone() {
            if !self.host_pinned()
                && let Some(text) =
                    unsupported_mount(&rt.helper_path, crate::privilege::ANTIDPI_CAPS).await
            {
                return self.fail_with(
                    connection.as_ref(),
                    ProcessError::AntiDpiMountUnsupported(text),
                );
            }
            #[cfg(any(test, feature = "test-utils"))]
            let pinned = self
                .host_probe
                .as_ref()
                .and_then(|probe| probe.helper_access.clone());
            #[cfg(not(any(test, feature = "test-utils")))]
            let pinned = None;
            let access = match pinned {
                Some(access) => access,
                None => {
                    let path = rt.helper_path.clone();
                    match tokio::task::spawn_blocking(move || crate::tun::helper_access(&path))
                        .await
                    {
                        Ok(access) => access,
                        Err(join) => {
                            return self.fail_with(
                                connection.as_ref(),
                                ProcessError::AntiDpiHelperAccessProbe {
                                    path: rt.helper_path.clone(),
                                    detail: join.to_string(),
                                },
                            );
                        }
                    }
                }
            };
            if let Some(error) = ProcessError::from_antidpi_access(&rt.helper_path, access) {
                return self.fail_with(connection.as_ref(), error);
            }
            // Root holds every capability in the process already, so probing
            // there only adds a failure mode. Pinned tests skip the probe too:
            // the probe program cannot be stubbed for the anti-DPI pair (see
            // the change design's security-scan constraints).
            let caps_gate = !self.host_pinned()
                && crate::privilege::caps_check_needed(nix::unistd::geteuid().as_raw());
            if caps_gate {
                let target = rt.helper_path.clone();
                let probe = tokio::task::spawn_blocking(move || {
                    crate::privilege::has_antidpi_caps(&target)
                })
                .await;
                let cap = match probe {
                    Ok(inner) => inner,
                    Err(join) => Err(crate::privilege::PrivilegeError::Probe(
                        rt.helper_path.clone(),
                        join.to_string(),
                    )),
                };
                match cap {
                    Ok(true) => {}
                    other => {
                        let error = match other {
                            Ok(false) => {
                                ProcessError::AntiDpiHelperCapabilityMissing(rt.helper_path.clone())
                            }
                            Err(e) => ProcessError::AntiDpiCapabilityProbe(e.to_string()),
                            Ok(true) => unreachable!(),
                        };
                        return self.fail_with(connection.as_ref(), error);
                    }
                }
            }
        }

        if let Some(rt) = self.tun.as_mut()
            && let Some(journal) = rt.journal.as_mut()
        {
            let identity = {
                let path = journal.path.clone();
                let session_id = journal.session_id.clone();
                match tokio::task::spawn_blocking(move || {
                    v2ray_rs_core::persistence::renew_completed_tun_session(&path, &session_id)
                })
                .await
                {
                    Ok(identity) => identity,
                    Err(join) => Err(std::io::Error::other(join).into()),
                }
            };
            let identity = match identity {
                Ok(identity) => identity,
                Err(e) => {
                    // A journal-identity failure blocks every candidate node,
                    // so it must not feed the per-candidate failover loop.
                    let error = ProcessError::TunJournal(e.to_string());
                    return self.fail_with(connection.as_ref(), error);
                }
            };
            if identity != journal.session_id {
                journal.session_id = identity;
                rt.device_ifindex = None;
            }
        }

        if connection.is_some() {
            self.current_connection = connection.clone();
        }

        self.state
            .transition(ProcessState::Starting, connection.clone())?;

        // Fail fast on a config the backend itself rejects, instead of burning
        // the crash-restart budget on immediate exits.
        if let Err(e) = self.check_config().await {
            let _ = self
                .state
                .transition(ProcessState::Error(e.to_string()), None);
            return Err(e);
        }

        let started = match self.launch().await {
            Ok(()) => self.wait_ready().await,
            err => err,
        };
        match started {
            Ok(()) => {
                self.state.transition(ProcessState::Running, connection)?;
                Ok(())
            }
            Err(e) => {
                let _ = self
                    .state
                    .transition(ProcessState::Error(e.to_string()), None);
                Err(e)
            }
        }
    }
    pub async fn stop(&mut self) -> Result<(), ProcessError> {
        match self.state() {
            // A stop that already reached Stopped can still carry an
            // unfinished anti-DPI obligation from a failed cleanup: retry it
            // instead of reporting a clean stop that left rules installed.
            // A start cancelled before its `Starting` transition never
            // launched, but the session already wrote this runtime's prepared
            // TUN journal; a finished stop can still carry an anti-DPI
            // obligation too, so the shared tail releases both.
            ProcessState::Stopped if self.child.is_none() => {
                self.finish_stop().await?;
                return Ok(());
            }
            // A preflight failure or crash-budget exhaustion leaves the manager
            // in Error with no child: release the engine and any routing state
            // and reach a terminal state instead of parking in Error.
            ProcessState::Error(_) if self.child.is_none() => {
                return self.finish_stop().await;
            }
            // A cancelled start or an exit whose wait future was dropped leaves
            // Starting/Running/Stopping with no child; finish the stop anyway.
            ProcessState::Stopping => {}
            _ => {
                self.state.transition(ProcessState::Stopping, None)?;
            }
        }
        self.graceful_stop().await;
        self.finish_stop().await
    }

    /// Shared stop tail: release the anti-DPI engine and rules, then the TUN
    /// routes, and reach `Stopped` only when both cleanups succeeded. Any
    /// failure transitions to `Error` so the caller keeps this manager alive
    /// for a retry instead of dropping it with kernel state still installed.
    pub(super) async fn finish_stop(&mut self) -> Result<(), ProcessError> {
        let antidpi = self.teardown_antidpi().await;
        let tun = self.teardown_tun().await;
        self.pid_file.remove().ok();
        // A manager that already sits in Error (start failure, earlier failed
        // cleanup) cannot re-enter Error through the state machine; the
        // returned error still carries the fresh cleanup failure.
        let report_error = |mgr: &mut Self, message: String| -> Result<(), ProcessError> {
            if !matches!(mgr.state.state(), ProcessState::Error(_)) {
                mgr.state.transition(ProcessState::Error(message), None)?;
            }
            Ok(())
        };
        match (tun, antidpi) {
            (Ok(()), Ok(())) => {
                // This arm is now reached from `Stopped` too: a second stop of
                // an already-stopped manager retries the cleanups, and a
                // `Stopped -> Stopped` transition is invalid.
                if self.state.state() != ProcessState::Stopped {
                    self.state.transition(ProcessState::Stopped, None)?;
                }
                Ok(())
            }
            (Err(tun), Ok(())) => {
                // A failed TUN teardown leaves strict-routing rules installed;
                // the Error state (not Stopped) keeps the TUN session marker
                // alive so startup recovery can clean the kernel up.
                report_error(self, format!("TUN teardown failed: {tun}"))?;
                Err(ProcessError::TunHelper(tun))
            }
            (Ok(()), Err(antidpi)) => {
                // The backend is down, but this manager's rules may remain:
                // surface the failure and keep the obligation for the next
                // stop to retry.
                report_error(self, format!("anti-DPI cleanup failed: {antidpi}"))?;
                Err(antidpi)
            }
            (Err(tun), Err(antidpi)) => {
                let error = ProcessError::CleanupFailed {
                    tun,
                    antidpi: antidpi.to_string(),
                };
                report_error(self, error.to_string())?;
                Err(error)
            }
        }
    }
    pub async fn restart(&mut self) -> Result<(), ProcessError> {
        if self.child.is_some() {
            self.stop().await?;
        }
        self.start().await
    }
    pub async fn shutdown(&mut self) -> Result<(), ProcessError> {
        self.shutdown_with(StopReason::UserStop).await
    }

    pub async fn shutdown_with(&mut self, reason: StopReason) -> Result<(), ProcessError> {
        self.stop_reason = reason;
        self.auto_restart = false;
        self.stop().await
    }

    pub fn check_orphaned(&self) -> std::io::Result<bool> {
        self.pid_file.check_and_kill_orphaned()
    }

    pub async fn wait_and_handle_exit(&mut self) -> Result<Option<i32>, ProcessError> {
        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };
        let status = match child.wait().await {
            Ok(status) => status,
            Err(err) => {
                self.cleanup_after_exit().await;
                self.write_exit_record(false, CRASH_REASON, None);
                let error = ProcessError::Wait(err);
                let _ = self
                    .state
                    .transition(ProcessState::Error(error.to_string()), None);
                return Err(error);
            }
        };
        let exit_code = status.code();

        self.cleanup_after_exit().await;

        self.state.emit(ProcessEvent::ProcessExited { exit_code });

        if self.state.state() == ProcessState::Running {
            self.handle_unexpected_exit(status).await;
        }

        Ok(exit_code)
    }

    /// Relaunches after a crash with the inputs the last start already checked:
    /// no version, capability or config probe, so a flapping backend cannot
    /// stall the restart on a slow preflight.
    pub(super) async fn respawn(&mut self) -> Result<(), ProcessError> {
        self.launch().await?;
        self.wait_ready().await?;
        self.state
            .transition(ProcessState::Running, self.current_connection.clone())?;
        Ok(())
    }

    // Runs after launch(), so an xray TUN start has already waited for the
    // device and run xray-up. Connecting only once the stability window has
    // passed keeps a backend that binds its inbounds and then dies in
    // post-start from counting as ready.
    pub(super) async fn wait_ready(&mut self) -> Result<(), ProcessError> {
        let Some(addr) = self.ready_probe else {
            return Ok(());
        };
        let probe_start = Instant::now();
        loop {
            let exited = match self.child.as_mut().map(Child::try_wait) {
                Some(Ok(status)) => status,
                Some(Err(err)) => {
                    self.stop_child(false, StopReason::StartFailed.as_str())
                        .await;
                    return Err(ProcessError::Wait(err));
                }
                None => None,
            };
            if let Some(status) = exited {
                self.cleanup_after_exit().await;
                let reason = self.exit_reason(&status);
                self.write_exit_record(false, StopReason::StartFailed.as_str(), Some(&status));
                return Err(ProcessError::ExitedBeforeReady(reason));
            }
            // Bounded: a connect to a non-loopback address behind a drop rule
            // would otherwise outlive ready_timeout.
            if probe_start.elapsed() >= STABILITY_WINDOW
                && let Ok(Ok(_)) =
                    tokio::time::timeout(READY_POLL_INTERVAL, TcpStream::connect(addr)).await
            {
                return Ok(());
            }
            if probe_start.elapsed() >= self.ready_timeout {
                self.stop_child(false, StopReason::StartFailed.as_str())
                    .await;
                return Err(ProcessError::ReadyTimeout {
                    addr,
                    timeout: self.ready_timeout,
                });
            }
            sleep(READY_POLL_INTERVAL).await;
        }
    }

    // A launch failure leaves routing state in place: during a respawn the
    // fail-closed routes must survive until the next attempt, and stop() owns
    // the teardown.
    pub(super) async fn launch(&mut self) -> Result<(), ProcessError> {
        self.write_session_record().await;
        if let Some(rt) = &self.tun
            && rt.needs_helper()
            && let Some(journal) = &rt.journal
        {
            let args = vec![
                "reserve".into(),
                "--iface".into(),
                rt.iface.clone(),
                "--journal".into(),
                journal.path.to_string_lossy().into_owned(),
                "--session-id".into(),
                journal.session_id.clone(),
            ];
            let run =
                crate::tun::run_helper(&rt.helper_path, &args, crate::tun::HELPER_TIMEOUT).await;
            self.log_helper("reserve", &run);
            if let Err(error) = run.result {
                return Err(ProcessError::TunHelper(error));
            }
        }
        if let Some(rt) = &self.tun
            && rt.backend == v2ray_rs_core::models::BackendType::SingBox
            && let Some(journal) = &rt.journal
        {
            let path = journal.path.clone();
            let session_id = journal.session_id.clone();
            match tokio::task::spawn_blocking(move || {
                v2ray_rs_core::persistence::record_singbox_launch(&path, &session_id)
            })
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(ProcessError::TunHelper(e.to_string())),
                Err(join) => return Err(ProcessError::TunHelper(join.to_string())),
            }
        }
        // Rules exist before any traffic flows: an install failure fails the
        // start before any backend spawn, so the user asked for protection and
        // never silently connects without it. A live engine from the previous
        // launch (crash respawn) means the rules are already in place — but an
        // engine whose exit is observed or unknown must never be reused
        // without fresh readiness.
        self.retain_live_engine().await;
        // An unverified engine exit from an earlier launch blocks both rule
        // churn and a replacement engine until operator recovery.
        if self.antidpi_exit_unverified {
            return Err(ProcessError::AntiDpiExitUnverified(
                "previous anti-DPI engine exit was never verified; no replacement may start".into(),
            ));
        }
        if self.antidpi.is_some() && self.antidpi_engine.is_none() {
            let rt = self.antidpi.clone().expect("antidpi runtime checked above");
            let session = self
                .antidpi_session_id
                .as_deref()
                .expect("session id allocated with the runtime");
            // Once an install is attempted, the stop path must try to remove:
            // a partially installed chain must not outlive the session.
            let already_owed = self.antidpi_rules_installed;
            self.antidpi_rules_installed = true;
            let run = crate::antidpi::antidpi_install_rules(&rt, session).await;
            self.log_helper("antidpi-install-rules", &run);
            if let Err(e) = run.result {
                // Exit status 20 is a verified pre-mutation refusal: this
                // request acquired nothing. Disarm only when nothing earlier
                // still owed cleanup; every other failure keeps the
                // obligation armed.
                if run.exit_code == Some(crate::antidpi::INSTALL_REFUSED_EXIT) && !already_owed {
                    self.antidpi_rules_installed = false;
                }
                return Err(ProcessError::AntiDpiHelper(e));
            }
        }
        let mut child = self.try_spawn().await?;

        if let Some(pid) = child.id() {
            let pid_file = self.pid_file.clone();
            let binary_path = self.binary_path.clone();
            let config_path = self.config_path.clone();
            let write = tokio::task::spawn_blocking(move || {
                pid_file.write(pid, &binary_path, &config_path)
            })
            .await;
            if let Err(err) = match write {
                Ok(result) => result,
                Err(join) => Err(std::io::Error::other(join)),
            } {
                log::warn!("failed to write pid ownership record: {err}");
            }
        }

        self.capture_output(&mut child);
        self.child = Some(child);

        // xray creates the device but does not program routes on Linux: wait for
        // the device, then drive the privileged helper. sing-box self-routes.
        if let Some(mut rt) = self.tun.clone()
            && rt.needs_helper()
        {
            let Some(ifindex) =
                crate::tun::wait_for_device(&rt.iface, crate::tun::DEVICE_TIMEOUT).await
            else {
                self.stop_child(false, StopReason::StartFailed.as_str())
                    .await;
                return Err(ProcessError::TunDeviceTimeout(rt.iface.clone()));
            };
            rt.device_ifindex = Some(ifindex);
            // Pin teardown to the exact device the backend created, so a name
            // reuse after a crash cannot redirect xray-down at another tunnel.
            if let Some(rt) = self.tun.as_mut() {
                rt.device_ifindex = Some(ifindex);
            }
            let run = crate::tun::xray_up(&rt).await;
            self.log_helper("xray-up", &run);
            if let Err(e) = run.result {
                self.stop_child(false, StopReason::StartFailed.as_str())
                    .await;
                return Err(ProcessError::TunHelper(e));
            }
        }

        // The engine starts last so the tunnel's route state has settled
        // first (with TUN off this lands directly after the backend spawn).
        if self.antidpi.is_some() && self.antidpi_engine.is_none() {
            let rt = self.antidpi.clone().expect("antidpi runtime checked above");
            let session = self
                .antidpi_session_id
                .clone()
                .expect("session id allocated with the runtime");
            if let Err(e) = self.start_antidpi_engine(&rt, &session).await {
                self.stop_child(false, StopReason::StartFailed.as_str())
                    .await;
                return Err(e);
            }
        }

        Ok(())
    }

    // Exactly one per launch, before the backend can produce output: the
    // counterpart every exit record refers back to.
    pub(super) async fn write_session_record(&mut self) {
        if self.log_writer.is_none() {
            return;
        }
        let version = self.backend_version().await;
        let version = strip_ansi(&version);
        let backend = self
            .backend
            .map(|b| b.to_string())
            .unwrap_or_else(|| "unknown".into());
        let node = self
            .current_connection
            .as_ref()
            .map(|c| truncate_reason(&c.node_name))
            .unwrap_or_else(|| "none".into());
        let tun = if self.tun.is_some() { "on" } else { "off" };
        let offset = local_utc_offset();
        let mut record = format!(
            "backend={backend} version={version} node={node} tun={tun} utc_offset={offset}"
        );
        if let Some(fields) = &self.session_fields {
            record.push(' ');
            record.push_str(&truncate_reason(fields));
        }
        write_stream_line(&self.log_writer, "session", &record);
        for line in &self.session_extra {
            write_stream_line(&self.log_writer, "dns", &truncate_reason(line));
        }
    }

    // Diagnostics-only probe for the session record: runs at most once per
    // manager, only when a backend log is attached (stub backends in tests are
    // never probed), and is bounded by CONFIG_CHECK_TIMEOUT so a wedged
    // `version` subcommand costs the first start once but never blocks it.
    pub(super) async fn backend_version(&mut self) -> String {
        if let Some(cached) = &self.cached_version {
            return cached.clone().unwrap_or_else(|| "unknown".into());
        }
        let probe = tokio::time::timeout(
            CONFIG_CHECK_TIMEOUT,
            Command::new(&self.binary_path)
                .arg("version")
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .map(|out| {
            let first = String::from_utf8_lossy(&out.stdout);
            let line = first.lines().next().unwrap_or("").trim();
            match parse_semver_triple(line) {
                Some(triple) => format_triple(triple),
                None if !line.is_empty() => truncate_reason(line),
                None => String::from("unknown"),
            }
        });
        self.cached_version = Some(probe.clone());
        probe.unwrap_or_else(|| "unknown".into())
    }

    // Only after cleanup_after_exit has drained the readers, so last_output is
    // the child's final line rather than a stale snapshot.
    pub(super) fn write_exit_record(
        &self,
        requested: bool,
        reason: &str,
        status: Option<&ExitStatus>,
    ) {
        if self.log_writer.is_none() {
            return;
        }
        let last = self.last_output_line().unwrap_or_else(|| "none".into());
        let err = self.last_error_line().unwrap_or_else(|| "none".into());
        write_stream_line(
            &self.log_writer,
            "exit",
            &format!(
                "requested={requested} reason={reason} {} crashes_in_window={} last_output={last} last_error={err}",
                exit_status_field(status),
                self.crash_times.len()
            ),
        );
    }

    /// Probes the backend's version for the minimum-version gates. Best-effort:
    /// a probe that fails, times out, exits non-zero or prints no version
    /// yields `None` and does not block the start — the pre-spawn config check
    /// still rejects configs the binary cannot handle.
    pub(super) async fn version_triple(&self) -> Option<(u32, u32, u32)> {
        let binary_path = self.binary_path.clone();
        let child = crate::spawn::spawn_with_etxtbsy_retry(move || {
            let mut cmd = Command::new(&binary_path);
            cmd.arg("version")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            cmd
        })
        .await
        .ok()?;
        let output = tokio::time::timeout(CONFIG_CHECK_TIMEOUT, child.wait_with_output())
            .await
            .ok()?
            .ok()?;
        if !output.status.success() {
            return None;
        }
        parse_semver_triple(&String::from_utf8_lossy(&output.stdout))
    }
    pub(super) async fn check_config(&self) -> Result<(), ProcessError> {
        let Some(backend) = self.backend else {
            return Ok(());
        };
        let args: &[&str] = match backend {
            BackendType::SingBox => &["check", "-c"],
            BackendType::Xray => &["run", "-test", "-c"],
            BackendType::V2ray => &["test", "-c"],
        };
        let binary_path = self.binary_path.clone();
        let config_path = self.config_path.clone();
        let geodata_dir = self.geodata_dir.clone();
        let child = crate::spawn::spawn_with_etxtbsy_retry(move || {
            let mut cmd = Command::new(&binary_path);
            cmd.args(args)
                .arg(&config_path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            if let Some(dir) = &geodata_dir {
                cmd.env("V2RAY_LOCATION_ASSET", dir);
                cmd.env("XRAY_LOCATION_ASSET", dir);
            }
            cmd
        })
        .await
        .map_err(ProcessError::Spawn)?;

        let output =
            match tokio::time::timeout(CONFIG_CHECK_TIMEOUT, child.wait_with_output()).await {
                Err(_) => return Err(ProcessError::ConfigCheck("config check timed out".into())),
                Ok(Err(e)) => return Err(ProcessError::Spawn(e)),
                Ok(Ok(output)) => output,
            };
        if output.status.success() {
            return Ok(());
        }

        // xray prints test failures to stdout, sing-box and v2ray to stderr.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let reason = last_nonempty_line(&stderr)
            .or_else(|| last_nonempty_line(&stdout))
            .map(truncate_reason)
            .unwrap_or_else(|| format!("exit code {:?}", output.status.code()));
        Err(ProcessError::ConfigCheck(reason))
    }

    // Retry on ETXTBSY which can occur on overlayfs (Docker containers)
    // when a binary is written and immediately executed
    pub(super) async fn try_spawn(&self) -> Result<Child, std::io::Error> {
        let binary_path = self.binary_path.clone();
        let config_path = self.config_path.clone();
        let geodata_dir = self.geodata_dir.clone();
        crate::spawn::spawn_with_etxtbsy_retry(move || {
            let mut cmd = Command::new(&binary_path);
            cmd.arg("run")
                .arg("-c")
                .arg(&config_path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);

            if let Some(dir) = &geodata_dir {
                cmd.env("V2RAY_LOCATION_ASSET", dir);
                cmd.env("XRAY_LOCATION_ASSET", dir);
            }
            cmd
        })
        .await
    }
    pub(super) async fn stop_child(&mut self, requested: bool, reason: &str) {
        let Some(child) = &mut self.child else {
            return;
        };

        if let Some(pid) = child.id() {
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
        }

        let mut status = tokio::time::timeout(STOP_TIMEOUT, child.wait())
            .await
            .ok()
            .and_then(Result::ok);
        if status.is_none() {
            child.kill().await.ok();
            status = child.wait().await.ok();
        }

        self.cleanup_after_exit().await;
        self.write_exit_record(requested, reason, status.as_ref());
    }

    pub(super) async fn handle_unexpected_exit(&mut self, status: ExitStatus) {
        let mut msg = self.exit_reason(&status);

        // Reaching here means the backend died while we still expected it
        // Running: stop() moves state to Stopping before killing, so a requested
        // stop never lands here. Every exit at this point is an unrequested
        // crash — including a signal death (OOM, segfault, external kill), which
        // reports exit_code == None on Unix and must not be mistaken for a clean
        // stop.
        //
        // Routing state stays in place across the respawn and on give-up: a
        // teardown here would drop the tunnel's fail-closed routes and leak
        // traffic; only stop() releases them.
        self.record_crash();
        self.write_exit_record(false, CRASH_REASON, Some(&status));

        if !self.auto_restart {
            let _ = self.state.transition(ProcessState::Error(msg), None);
            return;
        }

        loop {
            if self.crash_times.len() >= MAX_CRASHES {
                let _ = self.state.transition(
                    ProcessState::Error(format!(
                        "{MAX_CRASHES} crashes within {CRASH_WINDOW:?}: {msg}"
                    )),
                    None,
                );
                return;
            }

            log::info!(
                "backend exited unexpectedly ({}); respawn crash={}/{}",
                exit_status_field(Some(&status)),
                self.crash_times.len(),
                MAX_CRASHES
            );

            if self.state.state() == ProcessState::Running {
                let _ = self
                    .state
                    .transition(ProcessState::Starting, self.current_connection.clone());
            }

            sleep(self.restart_delay * self.crash_times.len() as u32).await;

            match self.respawn().await {
                Ok(()) => return,
                Err(e) => {
                    msg = format!("restart failed: {e}");
                    log::warn!("{msg}");
                    self.record_crash();
                }
            }
        }
    }

    pub(super) fn exit_reason(&self, status: &ExitStatus) -> String {
        let msg = match status.code() {
            Some(code) => format!("process exited with code {code}"),
            None => "process killed by signal".into(),
        };
        match self.last_output_line() {
            Some(reason) => format!("{msg}: {reason}"),
            None => msg,
        }
    }

    pub(super) fn record_crash(&mut self) {
        self.crash_times.push(Instant::now());
        self.crash_times.retain(|t| t.elapsed() < CRASH_WINDOW);
    }

    /// Kills the engine child and removes this manager's firewall rules,
    /// owner- and session-qualified. Engine-stop failures surface only as
    /// helper-stream log lines; a failed rule removal keeps the obligation
    /// and the helper's journal, and is returned so callers can end a stop
    /// or a failover with the host error instead of pretending the host is
    /// clean. Public because the session parks a failed candidate without
    /// stopping it: the parked manager's engine and rules protect a dead
    /// backend and would race the next candidate's queue binding.
    ///
    /// Signaling goes through the engine monitor, which owns the child
    /// handle; this task holds no numeric pid to signal. An unconfirmed exit
    /// (deadline, closed monitor, wait failure) returns an error and keeps
    /// the observation handle so a retry can request the stop again — the
    pub(super) async fn cleanup_after_exit(&mut self) {
        self.child = None;
        self.pid_file.remove().ok();
        // Child exit closed the pipes: let the readers drain what's left so the
        // crash reason reaches the buffer before it is read back.
        for handle in self.log_handles.drain(..) {
            let abort = handle.abort_handle();
            if tokio::time::timeout(LOG_DRAIN_TIMEOUT, handle)
                .await
                .is_err()
            {
                abort.abort();
            }
        }
    }

    /// Last non-empty captured line, preferring stderr over stdout.
    pub(super) fn last_output_line(&self) -> Option<String> {
        let buffer = self.log_buffer.lock().unwrap_or_else(|e| e.into_inner());
        let lines = buffer.last_n(50);
        let pick = |source: LogSource| {
            lines
                .iter()
                .rev()
                .find(|l| l.source == source && !l.content.trim().is_empty())
                .map(|l| truncate_reason(l.content.trim()))
        };
        pick(LogSource::Stderr).or_else(|| pick(LogSource::Stdout))
    }

    /// Last warning or error among recent output, so an access log that keeps
    /// running after a failure does not bury it.
    pub(super) fn last_error_line(&self) -> Option<String> {
        const MARKERS: [&str; 6] = ["[Warning]", "[Error]", "WARN", "ERROR", "FATAL", "panic"];
        let buffer = self.log_buffer.lock().unwrap_or_else(|e| e.into_inner());
        buffer.last_n(50).iter().rev().find_map(|l| {
            let plain = strip_ansi(&l.content);
            MARKERS
                .iter()
                .any(|m| plain.contains(m))
                .then(|| truncate_reason(plain.trim()))
        })
    }
}
