use super::*;

impl ProcessManager {
    // A launch failure leaves routing state in place: during a respawn the
    // fail-closed routes must survive until the next attempt, and stop() owns
    // the teardown.
    /// Keeps the engine handle only when the monitor is alive and no exit
    /// record has arrived. An empty exit receiver means the monitor still
    /// owns a possibly-running child: reuse it, its rules stay installed. A
    /// delivered record or a disconnected receiver (monitor gone without a
    /// record) drops the handle, so this launch re-installs and re-spawns:
    /// an engine whose exit state is observed or unknown must never be
    /// reused without fresh readiness.
    pub(super) fn retain_live_engine(&mut self) {
        if let Some(mut engine) = self.antidpi_engine.take()
            && let Err(mpsc::error::TryRecvError::Empty) = engine.exit_rx.try_recv()
        {
            self.antidpi_engine = Some(engine);
        }
    }

    /// Spawns the anti-DPI engine child by absolute path, waits for its ready
    /// sentinel on stdout (forwarding every line to the helper log stream as
    /// before), and only then hands the child to the monitor. A child that
    /// exits or stays silent before the sentinel is terminated and reaped
    /// here; the failure is a startup error, never a reported-running engine.
    pub(super) async fn start_antidpi_engine(
        &mut self,
        rt: &AntiDpiRuntime,
        session: &str,
    ) -> Result<(), ProcessError> {
        let mut child = Self::try_spawn_engine(rt, session)
            .await
            .map_err(|e| ProcessError::AntiDpiHelper(format!("engine spawn failed: {e}")))?;
        let (ready_tx, ready_rx) = mpsc::unbounded_channel();
        let (stderr_tx, mut stderr_rx) = mpsc::unbounded_channel();
        if let Some(stdout) = child.stdout.take() {
            self.push_helper_reader(stdout, Some(ready_tx));
        }
        if let Some(stderr) = child.stderr.take() {
            self.push_helper_reader(stderr, Some(stderr_tx));
        }
        match Self::wait_engine_ready(rt.queue, &mut child, ready_rx, &mut stderr_rx).await {
            Ok(()) => {
                let (stop_tx, exit_rx) = self.monitor_engine(child);
                self.antidpi_engine = Some(AntidpiEngine { stop_tx, exit_rx });
                Ok(())
            }
            Err(error) => {
                let exit = crate::antidpi::terminate_engine(child).await;
                let detail = format!(
                    "{}; termination reaped it ({})",
                    error,
                    exit_status_field(exit.status.as_ref())
                );
                log::error!("{detail}");
                self.push_notice(detail);
                Err(error)
            }
        }
    }

    /// Waits for the engine's exact stdout sentinel, bounded by
    /// `ANTIDPI_READY_TIMEOUT`. The exit status is read through `try_wait`, so
    /// the child is never owned by two waiters at once. A death is classified
    /// by the already-written output, never by a fixed sleep: the pipes are
    /// drained under the deadline, a complete exact sentinel followed by the
    /// exit is post-readiness death — readiness was proven, the start
    /// proceeds, and the monitor records the exit like any later engine
    /// death — and without the sentinel the startup error is returned.
    pub(super) async fn wait_engine_ready(
        queue: u16,
        child: &mut Child,
        mut ready_rx: mpsc::UnboundedReceiver<String>,
        stderr_rx: &mut mpsc::UnboundedReceiver<String>,
    ) -> Result<(), ProcessError> {
        let sentinel = crate::antidpi::ready_sentinel(queue);
        let deadline = tokio::time::Instant::now() + ANTIDPI_READY_TIMEOUT;
        let mut stdout_closed = false;
        let mut stderr_closed = false;
        let mut last_stderr: Option<String> = None;
        let mut sentinel_seen = false;
        loop {
            while let Ok(line) = ready_rx.try_recv() {
                if line == sentinel {
                    sentinel_seen = true;
                }
            }
            while let Ok(line) = stderr_rx.try_recv() {
                last_stderr = Some(line);
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    // The pipes close with the child; drain what the forwarder
                    // tasks already wrote, bounded by the same deadline, and
                    // classify the death by the sentinel.
                    while !(stdout_closed && stderr_closed)
                        && tokio::time::Instant::now() < deadline
                    {
                        tokio::select! {
                            line = ready_rx.recv(), if !stdout_closed => match line {
                                Some(line) => sentinel_seen |= line == sentinel,
                                None => stdout_closed = true,
                            },
                            line = stderr_rx.recv(), if !stderr_closed => match line {
                                Some(line) => last_stderr = Some(line),
                                None => stderr_closed = true,
                            },
                            else => break,
                        }
                    }
                    if sentinel_seen {
                        return Ok(());
                    }
                    return Err(ProcessError::AntiDpiStartupExited {
                        status: exit_status_field(Some(&status)),
                        stderr: last_stderr.map(|line| truncate_reason(&line)),
                    });
                }
                Ok(None) => {}
                Err(e) => {
                    return Err(ProcessError::AntiDpiStartupExited {
                        status: format!("wait failed: {e}"),
                        stderr: last_stderr.map(|line| truncate_reason(&line)),
                    });
                }
            }
            if sentinel_seen {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ProcessError::AntiDpiStartupTimeout {
                    queue,
                    timeout: ANTIDPI_READY_TIMEOUT,
                });
            }
            tokio::select! {
                line = ready_rx.recv(), if !stdout_closed => match line {
                    Some(line) if line == sentinel => return Ok(()),
                    Some(_) => {}
                    None => stdout_closed = true,
                },
                line = stderr_rx.recv(), if !stderr_closed => match line {
                    Some(line) => last_stderr = Some(line),
                    None => stderr_closed = true,
                },
                _ = sleep(ANTIDPI_ENGINE_POLL) => {}
            }
        }
    }

    /// Streams one engine output through the same path as one-shot helper
    /// output: the "helper" log stream, the shared `LogBuffer`, and
    /// `ProcessEvent::LogLine`, mirroring each line to `mirror` for the
    /// readiness scan. Tracked separately from the backend readers so a
    /// backend crash-respawn does not abort them while the engine lives on.
    pub(super) fn push_helper_reader(
        &mut self,
        stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
        mirror: Option<mpsc::UnboundedSender<String>>,
    ) {
        let tx = self.state.log_sender().clone();
        let buffer = Arc::clone(&self.log_buffer);
        let writer = self.log_writer.clone();
        self.antidpi_log_handles.push(tokio::spawn(async move {
            let mut reader = BufReader::new(stream);
            let mut buf = Vec::new();
            while let Ok(Some(line)) =
                read_bounded_line(&mut reader, &mut buf, MAX_LOG_LINE_BYTES).await
            {
                let line = strip_ansi(&line);
                if let Some(mirror) = &mirror {
                    let _ = mirror.send(line.to_string());
                }
                write_stream_line(&writer, "helper", &line);
                let log_line = LogLine::stderr(line);
                let _ = tx.send(ProcessEvent::LogLine(log_line.clone()));
                buffer
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(log_line);
            }
        }));
    }

    /// Watches the engine child: on a stop signal it terminates it exactly
    /// like the backend (SIGTERM → bounded wait → SIGKILL); on a self-exit it
    /// reports the death through the helper log stream — which the session
    /// forwards as log events — and records it, so the stop path removes the
    /// session's rules.
    pub(super) fn monitor_engine(
        &mut self,
        child: Child,
    ) -> (
        mpsc::UnboundedSender<()>,
        mpsc::UnboundedReceiver<EngineExit>,
    ) {
        let (stop_tx, mut stop_rx) = mpsc::unbounded_channel();
        let (exit_tx, exit_rx) = mpsc::unbounded_channel();
        let tx = self.state.log_sender().clone();
        let advisory_subs = Arc::clone(&self.advisory_subs);
        let buffer = Arc::clone(&self.log_buffer);
        let writer = self.log_writer.clone();
        tokio::spawn(async move {
            // Polling keeps both inputs on one task: `child.wait()` would own
            // the child for the engine's whole lifetime, and a stop request
            // arriving later could not reach it.
            let mut child = child;
            let exit = loop {
                // A stop request wins the race, so a requested stop is never
                // mistaken for an unexpected death. A closed stop channel is
                // also a shutdown request: the manager dropped its handle, so
                // only this task can terminate and reap the child — polling
                // an unreachable owner forever would orphan the engine.
                match stop_rx.try_recv() {
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    _ => break crate::antidpi::terminate_engine(child).await,
                }
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let exit = EngineExit {
                            status: Some(status),
                            wait_failed: false,
                        };
                        let status_field = exit_status_field(Some(&status));
                        let content =
                            format!("antidpi engine exited unexpectedly ({status_field})");
                        write_stream_line(&writer, "helper", &content);
                        let line = LogLine::stderr(content);
                        buffer
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(line.clone());
                        let _ = tx.send(ProcessEvent::LogLine(line));
                        // Advisory only: the stop path still owns cleanup, and
                        // no proxy state change rides on this signal. A closed
                        // advisory receiver means nobody is listening; the
                        // monitor keeps running either way.
                        publish_advisory(
                            &advisory_subs,
                            ProcessAdvisory::AntidpiDegraded {
                                detail: status_field,
                            },
                        );
                        break exit;
                    }
                    Ok(None) => {}
                    // A wait error is not an exit status: record it as an
                    // unconfirmed exit so the stop path refuses to report the
                    // queue free instead of logging "code=none" as a success.
                    Err(e) => {
                        let content = format!("antidpi engine wait failed: {e}");
                        write_stream_line(&writer, "helper", &content);
                        let line = LogLine::stderr(content);
                        buffer
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(line.clone());
                        let _ = tx.send(ProcessEvent::LogLine(line));
                        break EngineExit {
                            status: None,
                            wait_failed: true,
                        };
                    }
                }
                tokio::time::sleep(ANTIDPI_ENGINE_POLL).await;
            };
            let _ = exit_tx.send(exit);
        });
        (stop_tx, exit_rx)
    }

    pub(super) async fn try_spawn_engine(
        rt: &AntiDpiRuntime,
        session_id: &str,
    ) -> Result<Child, std::io::Error> {
        // Spawn by the same resolved absolute path the one-shot install-rules
        // and the preflight probes used: the verified binary is the binary
        // that starts, and the process-global environment is never mutated.
        let path = rt.helper_path.clone();
        let args = crate::antidpi::antidpi_run_args(rt, session_id);
        crate::spawn::spawn_with_etxtbsy_retry(move || {
            let mut cmd = Command::new(&path);
            cmd.args(&args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            cmd
        })
        .await
    }
    pub async fn teardown_antidpi(&mut self) -> Result<(), ProcessError> {
        let Some(rt) = self.antidpi.clone() else {
            return Ok(());
        };
        if let Some(mut engine) = self.antidpi_engine.take() {
            let _ = engine.stop_tx.send(());
            let record = tokio::time::timeout(ANTIDPI_EXIT_WAIT, engine.exit_rx.recv())
                .await
                .unwrap_or_default();
            if !exit_confirmed(record.as_ref()) {
                log::warn!("anti-DPI engine did not confirm its exit on stop; keeping its handle");
                self.antidpi_engine = Some(engine);
                return Err(ProcessError::AntiDpiHelper(
                    "anti-DPI engine did not confirm its exit within the stop deadline".into(),
                ));
            }
            // The engine's exit status belongs in the session record: it is
            // what a user compares against the helper's remove-rules output.
            if let Some(exit) = &record {
                self.push_notice(format!(
                    "antidpi engine stopped ({})",
                    exit_status_field(exit.status.as_ref())
                ));
            }
            for handle in self.antidpi_log_handles.drain(..) {
                let abort = handle.abort_handle();
                if tokio::time::timeout(LOG_DRAIN_TIMEOUT, handle)
                    .await
                    .is_err()
                {
                    abort.abort();
                }
            }
        }
        if self.antidpi_rules_installed {
            let session = self
                .antidpi_session_id
                .as_deref()
                .expect("session id allocated with the runtime");
            let run = crate::antidpi::antidpi_remove_rules(&rt, session).await;
            self.log_helper("antidpi-remove-rules", &run);
            match run.result {
                Ok(()) => {
                    self.antidpi_rules_installed = false;
                    Ok(())
                }
                // The obligation survives a failed removal: the next teardown
                // retries it instead of pretending the host is clean. The
                // helper's journal stays on disk with it.
                Err(e) => {
                    log::error!(
                        "anti-DPI remove-rules failed; firewall rules may remain installed: {e}"
                    );
                    self.push_notice(format!("anti-DPI remove-rules failed: {e}"));
                    Err(ProcessError::AntiDpiHelper(e))
                }
            }
        } else {
            Ok(())
        }
    }

    pub(super) async fn teardown_tun(&self) -> Result<(), String> {
        if let Some(rt) = &self.tun
            && let Some(journal) = &rt.journal
            && v2ray_rs_core::persistence::tun_session_replaced(&journal.path, &journal.session_id)
        {
            // Another session replaced this one in the journal, which only a
            // record without kernel state allows: nothing is left to release.
            return Ok(());
        }
        if let Some(rt) = self.tun.clone()
            && rt.needs_helper()
        {
            let run = crate::tun::xray_down(&rt).await;
            self.log_helper("xray-down", &run);
            return run.result;
        }
        if let Some(rt) = &self.tun
            && rt.backend == v2ray_rs_core::models::BackendType::SingBox
            && let Some(journal) = &rt.journal
        {
            tokio::time::timeout(
                crate::tun::HELPER_TIMEOUT,
                crate::tun::verify_singbox_stopped(&rt.iface),
            )
            .await
            .map_err(|_| "sing-box cleanup verification timed out".to_string())??;
            let path = journal.path.clone();
            let session_id = journal.session_id.clone();
            tokio::task::spawn_blocking(move || {
                v2ray_rs_core::persistence::complete_tun_session(&path, &session_id)
            })
            .await
            .map_err(|join| join.to_string())?
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub(super) fn log_helper(&self, verb: &str, run: &HelperRun) {
        let failure = run
            .result
            .as_ref()
            .err()
            .map(|e| format!("{verb} failed: {e}"));
        for raw in run
            .output
            .iter()
            .map(String::as_str)
            .chain(failure.as_deref())
        {
            let content = strip_ansi(raw);
            write_stream_line(&self.log_writer, "helper", &content);
            let line = LogLine::stderr(content);
            self.log_buffer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(line.clone());
            self.state.emit(ProcessEvent::LogLine(line));
        }
    }
}
