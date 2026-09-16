use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::broadcast;
use tokio::time::sleep;

use crate::log_buffer::{LogBuffer, LogLine, LogSource};
use crate::pid::PidFile;
use crate::state::{ProcessEvent, ProcessState, StateManager, TransitionError};
use crate::tun::{self, HelperRun, TunRuntime};
use v2ray_rs_core::models::{BackendType, ConnectionMetadata};
use v2ray_rs_core::rotating_log::RotatingFileWriter;

fn format_triple((major, minor, patch): (u32, u32, u32)) -> String {
    format!("{major}.{minor}.{patch}")
}

fn parse_semver_triple(text: &str) -> Option<(u32, u32, u32)> {
    text.split_whitespace().find_map(|tok| {
        let mut parts = tok.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        parts.next().is_none().then_some((major, minor, patch))
    })
}

const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const CRASH_RESTART_DELAY: Duration = Duration::from_secs(2);
const READY_TIMEOUT: Duration = Duration::from_secs(15);
const STABILITY_WINDOW: Duration = Duration::from_secs(1);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_CRASHES: usize = 3;
const CRASH_WINDOW: Duration = Duration::from_secs(60);
const CONFIG_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const LOG_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);
const REASON_MAX_CHARS: usize = 200;
const CRASH_REASON: &str = "crash";

/// Why a requested stop happened, recorded in the backend log's exit record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    UserStop,
    NodeSwitch,
    ApplyRestart,
    AppQuit,
    StartFailed,
}

impl StopReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            StopReason::UserStop => "user-stop",
            StopReason::NodeSwitch => "node-switch",
            StopReason::ApplyRestart => "apply-restart",
            StopReason::AppQuit => "app-quit",
            StopReason::StartFailed => "start-failed",
        }
    }
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("binary not found: {0}")]
    BinaryNotFound(PathBuf),
    #[error("config file missing: {0}")]
    ConfigMissing(PathBuf),
    #[error("spawn process: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("wait process: {0}")]
    Wait(std::io::Error),
    #[error("{0}")]
    Transition(#[from] TransitionError),
    #[error("backend {0} lacks CAP_NET_ADMIN required for TUN mode; grant TUN privileges first")]
    TunCapabilityMissing(PathBuf),
    #[error("could not verify TUN capabilities: {0}")]
    TunCapabilityProbe(String),
    #[error("TUN is unavailable: {0}")]
    TunMountUnsupported(String),
    #[error("TUN device {0} did not appear")]
    TunDeviceTimeout(String),
    #[error("TUN route helper failed: {0}")]
    TunHelper(String),
    #[error("route helper v2ray-rs-netctl not found")]
    TunHelperMissing,
    #[error("log out and back in to finish enabling TUN")]
    TunHelperRelogin,
    #[error(
        "route helper {0} lacks CAP_NET_ADMIN required for TUN mode; grant TUN privileges first"
    )]
    TunHelperCapabilityMissing(PathBuf),
    #[error(
        "installed {backend} {installed} is too old; {backend} {required} or newer is required"
    )]
    BackendTooOld {
        backend: BackendType,
        installed: String,
        required: String,
    },
    #[error("config rejected by backend: {0}")]
    ConfigCheck(String),
    #[error("{0}")]
    ExitedBeforeReady(String),
    #[error("backend did not accept connections on {addr} within {timeout:?}")]
    ReadyTimeout { addr: SocketAddr, timeout: Duration },
}

impl ProcessError {
    /// A host-level failure blocks every candidate node, so failover must
    /// stop: the machine itself lacks the capability or helper TUN needs, or
    /// has a backend too old to run at all. Everything else is per-candidate.
    pub fn is_host_level(&self) -> bool {
        matches!(
            self,
            ProcessError::TunCapabilityMissing(_)
                | ProcessError::TunCapabilityProbe(_)
                | ProcessError::TunMountUnsupported(_)
                | ProcessError::TunHelperMissing
                | ProcessError::TunHelperRelogin
                | ProcessError::TunHelperCapabilityMissing(_)
                | ProcessError::BackendTooOld { .. }
        )
    }
}

/// First Xray-core release shipping the `tun` inbound.
const XRAY_TUN_MIN_VERSION: (u32, u32, u32) = (26, 1, 13);
/// Oldest sing-box whose config schema the generator emits.
const SINGBOX_MIN_VERSION: (u32, u32, u32) = (1, 13, 0);
/// First Xray-core release with the fix for the TUN crash on quickly-closed
/// connections (gVisor returns a nil RemoteAddr, Xray-core #6364): versions
/// 26.1.13 through 26.6.22 panic and drop the tunnel until the crash-restart.
const XRAY_TUN_PANIC_FIX_VERSION: (u32, u32, u32) = (26, 6, 27);

pub struct ProcessManager {
    state: StateManager,
    log_buffer: Arc<Mutex<LogBuffer>>,
    pid_file: PidFile,
    child: Option<Child>,
    binary_path: PathBuf,
    config_path: PathBuf,
    geodata_dir: Option<PathBuf>,
    crash_times: Vec<Instant>,
    auto_restart: bool,
    restart_delay: Duration,
    ready_probe: Option<SocketAddr>,
    ready_timeout: Duration,
    log_handles: Vec<tokio::task::JoinHandle<()>>,
    current_connection: Option<ConnectionMetadata>,
    tun: Option<TunRuntime>,
    backend: Option<BackendType>,
    log_writer: Option<Arc<RotatingFileWriter>>,
    cached_version: Option<Option<String>>,
    stop_reason: StopReason,
    session_fields: Option<String>,
    #[cfg(any(test, feature = "test-utils"))]
    host_probe: Option<HostProbe>,
}

/// Pins the host facts the TUN preflight reads, so a test can reach the
/// capability verdict without depending on the mount table, the euid, or the
/// `getcap` and route helper installed on the machine.
#[cfg(any(test, feature = "test-utils"))]
#[derive(Debug, Clone)]
pub struct HostProbe {
    pub getcap: PathBuf,
    pub helper: PathBuf,
}

impl ProcessManager {
    pub fn new(
        binary_path: PathBuf,
        config_path: PathBuf,
        pid_path: PathBuf,
        geodata_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            state: StateManager::new(),
            log_buffer: Arc::new(Mutex::new(LogBuffer::new())),
            pid_file: PidFile::new(pid_path),
            child: None,
            binary_path,
            config_path,
            geodata_dir,
            crash_times: Vec::new(),
            auto_restart: true,
            restart_delay: CRASH_RESTART_DELAY,
            ready_probe: None,
            ready_timeout: READY_TIMEOUT,
            log_handles: Vec::new(),
            current_connection: None,
            tun: None,
            backend: None,
            log_writer: None,
            cached_version: None,
            stop_reason: StopReason::UserStop,
            session_fields: None,
            #[cfg(any(test, feature = "test-utils"))]
            host_probe: None,
        }
    }

    /// Skips the mount gate, always runs the capability gate, probes with
    /// `probe.getcap`, and points an attached TUN runtime at `probe.helper`.
    /// Call after `with_tun`.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn with_host_probe(mut self, probe: HostProbe) -> Self {
        if let Some(rt) = &mut self.tun {
            rt.helper_path = probe.helper.clone();
        }
        self.host_probe = Some(probe);
        self
    }

    fn host_pinned(&self) -> bool {
        #[cfg(any(test, feature = "test-utils"))]
        return self.host_probe.is_some();
        #[cfg(not(any(test, feature = "test-utils")))]
        false
    }

    fn getcap_program(&self) -> PathBuf {
        #[cfg(any(test, feature = "test-utils"))]
        if let Some(probe) = &self.host_probe {
            return probe.getcap.clone();
        }
        PathBuf::from("getcap")
    }

    /// Attaches TUN runtime details so start/stop become TUN-aware.
    pub fn with_tun(mut self, tun: Option<TunRuntime>) -> Self {
        self.tun = tun;
        self
    }

    /// Holds a start in `Starting` until the backend accepts connections on
    /// `addr` and has stayed up for the stability window.
    pub fn with_ready_probe(mut self, addr: SocketAddr) -> Self {
        self.ready_probe = Some(addr);
        self
    }

    /// Enables the pre-spawn config check using the backend's own validator.
    pub fn with_backend(mut self, backend: BackendType) -> Self {
        self.backend = Some(backend);
        self
    }

    /// Attaches a shared writer so backend output, session and exit records
    /// land in the backend log file.
    pub fn with_log_file(mut self, writer: Option<Arc<RotatingFileWriter>>) -> Self {
        self.log_writer = writer;
        self
    }

    /// Appends caller-decided `key=value` fields to every session record.
    pub fn with_session_fields(mut self, fields: String) -> Self {
        self.session_fields = Some(fields);
        self
    }

    pub fn state(&self) -> ProcessState {
        self.state.state()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProcessEvent> {
        self.state.subscribe()
    }

    pub fn subscribe_logs(&self) -> broadcast::Receiver<ProcessEvent> {
        self.state.subscribe_logs()
    }

    pub fn log_buffer(&self) -> &Arc<Mutex<LogBuffer>> {
        &self.log_buffer
    }

    pub fn set_auto_restart(&mut self, enabled: bool) {
        self.auto_restart = enabled;
    }

    pub async fn start(&mut self) -> Result<(), ProcessError> {
        self.start_with_connection(None).await
    }

    /// Terminal tail of a failed preflight: step through `Starting`, land in
    /// `Error` with the error's own text, and hand the error to `return`. The
    /// `Starting` hop keeps the transition history of a preflight rejection
    /// identical to one that fails after launch.
    fn fail_with(
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
                && !crate::privilege::file_caps_supported(
                    self.binary_path.parent().unwrap_or(Path::new("/")),
                )
            {
                let error = ProcessError::TunMountUnsupported(
                    crate::privilege::PrivilegeError::Unsupported {
                        path: self.binary_path.clone(),
                        caps: crate::privilege::BACKEND_CAPS.to_string(),
                    }
                    .to_string(),
                );
                return self.fail_with(connection.as_ref(), error);
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
                if !installed.is_absolute() || !installed.exists() {
                    return self.fail_with(connection.as_ref(), ProcessError::TunHelperMissing);
                }
                // A freshly granted relocated helper is group-executable
                // only; a session that has not picked up its new group yet
                // cannot run it at all.
                if nix::unistd::access(installed, nix::unistd::AccessFlags::X_OK).is_err() {
                    return self.fail_with(connection.as_ref(), ProcessError::TunHelperRelogin);
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
            ProcessState::Stopped if self.child.is_none() => return Ok(()),
            // A preflight failure or crash-budget exhaustion leaves the manager
            // in Error with no child: release any routing state and reach a
            // terminal state instead of parking in Error.
            ProcessState::Error(_) if self.child.is_none() => {
                self.teardown_tun().await;
                self.state.transition(ProcessState::Stopped, None)?;
                return Ok(());
            }
            // A cancelled start or an exit whose wait future was dropped leaves
            // Starting/Running/Stopping with no child; finish the stop anyway.
            ProcessState::Stopping => {}
            _ => {
                self.state.transition(ProcessState::Stopping, None)?;
            }
        }
        self.graceful_stop().await;
        self.teardown_tun().await;
        self.state.transition(ProcessState::Stopped, None)?;
        self.pid_file.remove().ok();
        Ok(())
    }

    pub async fn restart(&mut self) -> Result<(), ProcessError> {
        if self.child.is_some() {
            self.stop().await?;
        }
        self.start().await
    }
    pub async fn shutdown(&mut self) {
        self.shutdown_with(StopReason::UserStop).await;
    }

    pub async fn shutdown_with(&mut self, reason: StopReason) {
        self.stop_reason = reason;
        self.auto_restart = false;
        let _ = self.stop().await;
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
    async fn respawn(&mut self) -> Result<(), ProcessError> {
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
    async fn wait_ready(&mut self) -> Result<(), ProcessError> {
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
    async fn launch(&mut self) -> Result<(), ProcessError> {
        self.write_session_record().await;
        let mut child = self.try_spawn().await?;

        if let Some(pid) = child.id()
            && let Err(err) = self
                .pid_file
                .write(pid, &self.binary_path, &self.config_path)
        {
            log::warn!("failed to write pid ownership record: {err}");
        }

        self.capture_output(&mut child);
        self.child = Some(child);

        // xray creates the device but does not program routes on Linux: wait for
        // the device, then drive the privileged helper. sing-box self-routes.
        if let Some(rt) = self.tun.clone()
            && rt.needs_helper()
        {
            if !tun::wait_for_device(&rt.iface, tun::DEVICE_TIMEOUT).await {
                self.stop_child(false, StopReason::StartFailed.as_str())
                    .await;
                return Err(ProcessError::TunDeviceTimeout(rt.iface.clone()));
            }
            let run = tun::xray_up(&rt).await;
            self.log_helper("xray-up", &run);
            if let Err(e) = run.result {
                self.stop_child(false, StopReason::StartFailed.as_str())
                    .await;
                return Err(ProcessError::TunHelper(e));
            }
        }

        Ok(())
    }

    // Exactly one per launch, before the backend can produce output: the
    // counterpart every exit record refers back to.
    async fn write_session_record(&mut self) {
        if self.log_writer.is_none() {
            return;
        }
        let version = self.backend_version().await;
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
        let mut record = format!("backend={backend} version={version} node={node} tun={tun}");
        if let Some(fields) = &self.session_fields {
            record.push(' ');
            record.push_str(&truncate_reason(fields));
        }
        write_stream_line(&self.log_writer, "session", &record);
    }

    // Diagnostics-only probe for the session record: runs at most once per
    // manager, only when a backend log is attached (stub backends in tests are
    // never probed), and is bounded by CONFIG_CHECK_TIMEOUT so a wedged
    // `version` subcommand costs the first start once but never blocks it.
    async fn backend_version(&mut self) -> String {
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
    fn write_exit_record(&self, requested: bool, reason: &str, status: Option<&ExitStatus>) {
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
    async fn version_triple(&self) -> Option<(u32, u32, u32)> {
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

    fn push_notice(&self, content: String) {
        write_stream_line(&self.log_writer, "notice", &content);
        let line = LogLine::stderr(content);
        self.log_buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(line.clone());
        self.state.emit(ProcessEvent::LogLine(line));
    }

    async fn check_config(&self) -> Result<(), ProcessError> {
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
    async fn try_spawn(&self) -> Result<Child, std::io::Error> {
        let binary_path = self.binary_path.clone();
        let config_path = self.config_path.clone();
        let geodata_dir = self.geodata_dir.clone();
        crate::spawn::spawn_with_etxtbsy_retry(move || {
            let mut cmd = Command::new(&binary_path);
            cmd.arg("run")
                .arg("-c")
                .arg(&config_path)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            if let Some(dir) = &geodata_dir {
                cmd.env("V2RAY_LOCATION_ASSET", dir);
                cmd.env("XRAY_LOCATION_ASSET", dir);
            }
            cmd
        })
        .await
    }

    fn capture_output(&mut self, child: &mut Child) {
        if let Some(stdout) = child.stdout.take() {
            let tx = self.state.log_sender().clone();
            let buffer = Arc::clone(&self.log_buffer);
            let writer = self.log_writer.clone();
            self.log_handles.push(tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    write_stream_line(&writer, "stdout", &line);
                    let log_line = LogLine::stdout(&line);
                    let _ = tx.send(ProcessEvent::LogLine(log_line.clone()));
                    buffer
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(log_line);
                }
            }));
        }

        if let Some(stderr) = child.stderr.take() {
            let tx = self.state.log_sender().clone();
            let buffer = Arc::clone(&self.log_buffer);
            let writer = self.log_writer.clone();
            self.log_handles.push(tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    write_stream_line(&writer, "stderr", &line);
                    let log_line = LogLine::stderr(&line);
                    let _ = tx.send(ProcessEvent::LogLine(log_line.clone()));
                    buffer
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(log_line);
                }
            }));
        }
    }

    async fn graceful_stop(&mut self) {
        self.stop_child(true, self.stop_reason.as_str()).await;
    }

    async fn stop_child(&mut self, requested: bool, reason: &str) {
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

    async fn handle_unexpected_exit(&mut self, status: ExitStatus) {
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

    fn exit_reason(&self, status: &ExitStatus) -> String {
        let msg = match status.code() {
            Some(code) => format!("process exited with code {code}"),
            None => "process killed by signal".into(),
        };
        match self.last_output_line() {
            Some(reason) => format!("{msg}: {reason}"),
            None => msg,
        }
    }

    fn record_crash(&mut self) {
        self.crash_times.push(Instant::now());
        self.crash_times.retain(|t| t.elapsed() < CRASH_WINDOW);
    }

    async fn teardown_tun(&self) {
        if let Some(rt) = self.tun.clone()
            && rt.needs_helper()
        {
            let run = tun::xray_down(&rt).await;
            self.log_helper("xray-down", &run);
        }
    }

    fn log_helper(&self, verb: &str, run: &HelperRun) {
        let failure = run
            .result
            .as_ref()
            .err()
            .map(|e| format!("{verb} failed: {e}"));
        for content in run.output.iter().cloned().chain(failure) {
            write_stream_line(&self.log_writer, "helper", &content);
            let line = LogLine::stderr(content);
            self.log_buffer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(line.clone());
            self.state.emit(ProcessEvent::LogLine(line));
        }
    }

    async fn cleanup_after_exit(&mut self) {
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
    fn last_output_line(&self) -> Option<String> {
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
    fn last_error_line(&self) -> Option<String> {
        const MARKERS: [&str; 6] = ["[Warning]", "[Error]", "WARN", "ERROR", "FATAL", "panic"];
        let buffer = self.log_buffer.lock().unwrap_or_else(|e| e.into_inner());
        buffer.last_n(50).iter().rev().find_map(|l| {
            let plain = without_ansi(&l.content);
            MARKERS
                .iter()
                .any(|m| plain.contains(m))
                .then(|| truncate_reason(plain.trim()))
        })
    }
}

fn too_old(
    backend: BackendType,
    installed: (u32, u32, u32),
    required: (u32, u32, u32),
) -> ProcessError {
    ProcessError::BackendTooOld {
        backend,
        installed: format_triple(installed),
        required: format_triple(required),
    }
}

fn unreadable_version(backend: BackendType) -> String {
    format!("warning: could not read {backend} version; minimum-version check skipped")
}

fn last_nonempty_line(text: &str) -> Option<&str> {
    text.lines().rev().map(str::trim).find(|l| !l.is_empty())
}

fn truncate_reason(line: &str) -> String {
    // Records are single-line: newlines in attacker-controlled free text
    // (subscription node names, backend output) must not forge extra lines.
    let sanitized: String = line
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    if sanitized.chars().count() > REASON_MAX_CHARS {
        let cut: String = sanitized.chars().take(REASON_MAX_CHARS).collect();
        format!("{cut}…")
    } else {
        sanitized
    }
}

// sing-box colours its level tags; strip CSI sequences before matching them.
fn without_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        if chars.clone().next() == Some('[') {
            chars.next();
            for c in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

fn exit_status_field(status: Option<&ExitStatus>) -> String {
    use std::os::unix::process::ExitStatusExt;
    let Some(status) = status else {
        return "code=none".to_string();
    };
    if let Some(code) = status.code() {
        format!("code={code}")
    } else if let Some(signal) = status.signal() {
        format!("signal={signal}")
    } else {
        "code=none".to_string()
    }
}

// Mirrors one backend or helper line into the log file, ordered before the
// buffer push: the broadcast channel can lag or drop, the file must not. The
// writer is sync and infallible, so callers never await on it.
fn write_stream_line(writer: &Option<Arc<RotatingFileWriter>>, stream: &str, line: &str) {
    if let Some(writer) = writer {
        writer.append_line(stream, line);
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            if let Some(pid) = child.id() {
                log::warn!("ProcessManager dropped with live child (pid {pid}); sending SIGKILL");
                let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
                let _ = child.try_wait();
            }
            self.pid_file.remove().ok();
            for handle in self.log_handles.drain(..) {
                handle.abort();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tun::TunRuntime;
    use v2ray_rs_core::models::BackendType;
    use v2ray_rs_core::rotating_log::DEFAULT_MAX_BYTES;

    fn write_script(dir: &std::path::Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("backend");
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn manager_for(dir: &tempfile::TempDir, script_body: &str) -> ProcessManager {
        let config = dir.path().join("config.json");
        std::fs::write(&config, "{}").unwrap();
        let binary = write_script(dir.path(), script_body);
        ProcessManager::new(binary, config, dir.path().join("backend.pid"), None)
    }

    const VERSION_STUB: &str = "if [ \"$1\" = version ]; then echo 'Xray 26.3.27 (Xray, Penetrates Everything.)'; exit 0; fi\n";
    const SINGBOX_VERSION_STUB: &str =
        "[ \"$1\" = version ] && { echo 'sing-box version 1.13.0'; exit 0; }\n";
    fn backend_log(dir: &std::path::Path) -> Arc<RotatingFileWriter> {
        Arc::new(RotatingFileWriter::open(dir.join("backend.log"), DEFAULT_MAX_BYTES).unwrap())
    }

    #[tokio::test]
    async fn config_check_failure_prevents_spawn() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(
            &dir,
            &format!("{SINGBOX_VERSION_STUB}if [ \"$1\" = check ]; then echo 'FATAL bad dns config' >&2; exit 1; fi\nexec sleep 30\n"),
        )
        .with_backend(BackendType::SingBox);

        let result = mgr.start_with_connection(None).await;
        assert!(
            matches!(result, Err(ProcessError::ConfigCheck(_))),
            "expected ConfigCheck error, got {result:?}"
        );
        match mgr.state() {
            ProcessState::Error(msg) => assert!(msg.contains("FATAL bad dns config"), "{msg}"),
            other => panic!("expected Error state, got {other:?}"),
        }
        assert!(mgr.child.is_none(), "no backend should have been spawned");
    }

    #[tokio::test]
    async fn config_check_success_starts_backend() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(
            &dir,
            &format!("{SINGBOX_VERSION_STUB}[ \"$1\" = check ] && exit 0\nexec sleep 30\n"),
        )
        .with_backend(BackendType::SingBox);

        mgr.start_with_connection(None).await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Running);
        mgr.stop().await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Stopped);
    }

    #[tokio::test]
    async fn crash_error_includes_last_stderr_line() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, "echo 'fatal: tls handshake exploded' >&2\nexit 3\n");
        mgr.set_auto_restart(false);

        mgr.start_with_connection(None).await.unwrap();
        let code = mgr.wait_and_handle_exit().await.unwrap();
        assert_eq!(code, Some(3));
        match mgr.state() {
            ProcessState::Error(msg) => {
                assert!(msg.contains("code 3"), "{msg}");
                assert!(msg.contains("tls handshake exploded"), "{msg}");
            }
            other => panic!("expected Error state, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn startup_failure_writes_one_session_and_no_crash() {
        let dir = tempfile::TempDir::new().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut mgr = manager_for(&dir, "echo FATAL >&2\nexit 1\n")
            .with_log_file(Some(backend_log(dir.path())))
            .with_ready_probe(listener.local_addr().unwrap());
        mgr.restart_delay = Duration::from_millis(50);

        let result = mgr.start().await;
        assert!(
            matches!(result, Err(ProcessError::ExitedBeforeReady(_))),
            "{result:?}"
        );

        let log = dir.path().join("backend.log");
        let sessions = |lines: &[String]| lines.iter().filter(|l| l.contains(" session ")).count();
        let lines = read_lines(&log);
        assert_eq!(sessions(&lines), 1, "{lines:#?}");
        let exit = lines
            .iter()
            .find(|l| l.contains(" exit "))
            .unwrap_or_else(|| panic!("no exit record: {lines:#?}"));
        for field in [
            "exit requested=false reason=start-failed",
            "code=1",
            "crashes_in_window=0",
            "last_output=FATAL",
        ] {
            assert!(exit.contains(field), "{field} missing in {exit}");
        }

        sleep(Duration::from_secs(3)).await;
        assert_eq!(sessions(&read_lines(&log)), 1);
        assert!(matches!(mgr.state(), ProcessState::Error(_)));
    }

    #[tokio::test]
    async fn respawn_readiness_failure_counts_as_crash() {
        let dir = tempfile::TempDir::new().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let runs = dir.path().join("runs");
        let script = format!(
            "echo run >> {runs}\n[ $(wc -l < {runs}) -le 1 ] && {{ sleep 2; exit 1; }}\nexit 1\n",
            runs = runs.display()
        );
        let mut mgr = manager_for(&dir, &script).with_ready_probe(listener.local_addr().unwrap());
        mgr.restart_delay = Duration::from_millis(50);

        mgr.start().await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Running);
        while mgr.state() == ProcessState::Running {
            mgr.wait_and_handle_exit().await.unwrap();
        }

        match mgr.state() {
            ProcessState::Error(msg) => {
                assert!(msg.contains("3 crashes within 60s"), "{msg}");
                assert!(msg.contains("restart failed"), "{msg}");
            }
            other => panic!("expected Error state, got {other:?}"),
        }
        assert_eq!(mgr.crash_times.len(), MAX_CRASHES);
        assert_eq!(read_lines(&runs).len(), MAX_CRASHES);
    }

    #[tokio::test]
    async fn backend_log_captures_all_lines_under_load() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}seq 1 20000; exec sleep 30\n"))
            .with_log_file(Some(backend_log(dir.path())));

        mgr.start().await.unwrap();
        let lines = wait_for_lines(&dir.path().join("backend.log"), 20_001).await;
        assert_eq!(
            lines.len(),
            20_001,
            "session record plus 20,000 stdout lines"
        );
        assert!(lines[0].contains("session backend="), "{}", lines[0]);
        for (i, line) in lines[1..].iter().enumerate() {
            assert!(
                line.ends_with(&format!("stdout {}", i + 1)),
                "out of order at position {}: {line}",
                i + 1
            );
        }
        mgr.stop().await.unwrap();
    }

    #[tokio::test]
    async fn session_record_precedes_spawn() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(
            &dir,
            &format!("{VERSION_STUB}echo out-line\necho err-line >&2\nexec sleep 30\n"),
        )
        .with_log_file(Some(backend_log(dir.path())));

        mgr.start().await.unwrap();
        let lines = wait_for_lines(&dir.path().join("backend.log"), 3).await;
        let session = &lines[0];
        assert!(session.contains("session backend=unknown"), "{session}");
        assert!(session.contains("version=26.3.27"), "{session}");
        assert!(session.contains("node=none"), "{session}");
        assert!(session.contains("tun=off"), "{session}");
        assert!(
            lines[1..].iter().any(|l| l.contains("stdout out-line")),
            "{lines:?}"
        );
        assert!(
            lines[1..].iter().any(|l| l.contains("stderr err-line")),
            "{lines:?}"
        );
        mgr.stop().await.unwrap();
    }

    #[tokio::test]
    async fn session_record_appends_caller_fields() {
        let dir = tempfile::TempDir::new().unwrap();
        let fields = "hijack=hijack capture_dns=true strict=false nodes_pinned=true profile=app";
        let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
            .with_log_file(Some(backend_log(dir.path())))
            .with_session_fields(fields.into());

        mgr.start().await.unwrap();
        mgr.stop().await.unwrap();

        let lines = read_lines(&dir.path().join("backend.log"));
        let sessions: Vec<&String> = lines.iter().filter(|l| l.contains(" session ")).collect();
        assert_eq!(sessions.len(), 1, "{lines:?}");
        assert!(
            sessions[0].contains("tun=off hijack=hijack"),
            "{}",
            sessions[0]
        );
        assert!(sessions[0].ends_with(fields), "{}", sessions[0]);
    }

    #[tokio::test]
    async fn session_fields_cannot_forge_a_second_line() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
            .with_log_file(Some(backend_log(dir.path())))
            .with_session_fields("hijack=hijack\n2026-01-01 session forged=true\rx=y".into());

        mgr.start().await.unwrap();
        mgr.stop().await.unwrap();

        let lines = read_lines(&dir.path().join("backend.log"));
        let sessions: Vec<&String> = lines.iter().filter(|l| l.contains(" session ")).collect();
        assert_eq!(sessions.len(), 1, "{lines:?}");
        assert!(
            sessions[0].contains("hijack=hijack 2026-01-01 session forged=true x=y"),
            "{}",
            sessions[0]
        );
    }

    #[tokio::test]
    async fn session_record_sanitizes_node_name_newlines() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
            .with_log_file(Some(backend_log(dir.path())));
        let connection: ConnectionMetadata = serde_json::from_str(
            r#"{
                "node_ref": {"type": "manual", "node_id": "00000000-0000-0000-0000-000000000000"},
                "source": "Manual",
                "source_id": "00000000-0000-0000-0000-000000000001",
                "node_name": "evil\nnode\rname",
                "node_address": "example.com",
                "node_port": 443,
                "backend": "xray",
                "strategy": "list-order",
                "latency_ms": null,
                "connected_since": "2026-01-01T00:00:00Z"
            }"#,
        )
        .unwrap();

        mgr.start_with_connection(Some(connection)).await.unwrap();
        mgr.stop().await.unwrap();

        let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
        let sessions: Vec<&String> = lines
            .iter()
            .filter(|l| l.contains("session backend="))
            .collect();
        assert_eq!(
            sessions.len(),
            1,
            "a newline in the node name must not forge extra record lines: {lines:?}"
        );
        assert!(
            sessions[0].contains("node=evil node name"),
            "{}",
            sessions[0]
        );
    }

    #[tokio::test]
    async fn exit_record_marks_requested_stop() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
            .with_log_file(Some(backend_log(dir.path())));

        mgr.start().await.unwrap();
        mgr.stop().await.unwrap();
        let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
        assert!(lines[0].contains("session backend="), "{}", lines[0]);
        let exit = lines
            .iter()
            .find(|l| l.contains("exit requested="))
            .expect("exit record");
        assert!(
            exit.contains("exit requested=true reason=user-stop"),
            "{exit}"
        );
        assert!(exit.contains("signal=15"), "{exit}");
        assert!(exit.contains("crashes_in_window=0"), "{exit}");
    }

    #[tokio::test]
    async fn exit_record_reason_per_stop_reason() {
        for reason in [
            StopReason::NodeSwitch,
            StopReason::ApplyRestart,
            StopReason::AppQuit,
        ] {
            let dir = tempfile::TempDir::new().unwrap();
            let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
                .with_log_file(Some(backend_log(dir.path())));

            mgr.start().await.unwrap();
            mgr.shutdown_with(reason).await;
            let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
            let exits: Vec<_> = lines.iter().filter(|l| l.contains(" exit ")).collect();
            assert_eq!(exits.len(), 1, "{lines:#?}");
            let expected = format!("exit requested=true reason={reason}");
            assert!(exits[0].contains(&expected), "{}", exits[0]);
        }
    }

    #[tokio::test]
    async fn exit_record_marks_start_failed_on_tun_device_timeout() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, &format!("{VERSION_STUB}exec sleep 30\n"))
            .with_log_file(Some(backend_log(dir.path())));
        let (helper, _) = stub_helper(dir.path(), "[ \"$1\" = xray-up ] && exit 1\nexit 0\n");
        mgr.tun = Some(xray_on_lo(helper));

        let result = mgr.launch().await;
        assert!(
            matches!(result, Err(ProcessError::TunHelper(_))),
            "{result:?}"
        );
        assert!(mgr.child.is_none());

        let lines = read_lines(&dir.path().join("backend.log"));
        let exits: Vec<_> = lines
            .iter()
            .filter(|l| l.contains(" exit requested="))
            .collect();
        assert_eq!(exits.len(), 1, "{lines:#?}");
        assert!(
            exits[0].contains("exit requested=false reason=start-failed"),
            "{}",
            exits[0]
        );
    }

    #[tokio::test]
    async fn exit_record_marks_unrequested_crash() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(
            &dir,
            &format!("{VERSION_STUB}echo 'fatal: tls handshake exploded' >&2\nexit 3\n"),
        )
        .with_log_file(Some(backend_log(dir.path())));
        mgr.set_auto_restart(false);

        mgr.start().await.unwrap();
        mgr.wait_and_handle_exit().await.unwrap();
        let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
        assert!(lines[0].contains("session backend="), "{}", lines[0]);
        let exit = lines
            .iter()
            .find(|l| l.contains("exit requested="))
            .expect("exit record");
        assert!(exit.contains("exit requested=false reason=crash"), "{exit}");
        assert!(exit.contains("code=3"), "{exit}");
        assert!(exit.contains("crashes_in_window=1"), "{exit}");
        assert!(
            exit.contains("last_output=fatal: tls handshake exploded"),
            "{exit}"
        );
    }

    async fn crash_exit_record(dir: &tempfile::TempDir, body: &str) -> (String, String) {
        let mut mgr = manager_for(dir, &format!("{VERSION_STUB}{body}"))
            .with_log_file(Some(backend_log(dir.path())));
        mgr.set_auto_restart(false);

        mgr.start().await.unwrap();
        mgr.wait_and_handle_exit().await.unwrap();
        let ProcessState::Error(msg) = mgr.state() else {
            panic!("expected Error state, got {:?}", mgr.state());
        };
        let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
        let exit = lines
            .iter()
            .find(|l| l.contains(" exit requested="))
            .unwrap_or_else(|| panic!("no exit record: {lines:#?}"))
            .clone();
        (exit, msg)
    }

    #[tokio::test]
    async fn last_error_survives_access_noise() {
        let dir = tempfile::TempDir::new().unwrap();
        let access = "2026/09/16 10:00:01 from 127.0.0.1:50002 accepted tcp:example.com:443 [socks -> proxy]";
        let body = format!(
            "echo '[Warning] cert about to expire'\necho '2026/09/16 10:00:00 from 127.0.0.1:50001 accepted tcp:example.org:443 [socks -> proxy]'\necho '{access}'\nexit 3\n"
        );
        let (exit, msg) = crash_exit_record(&dir, &body).await;
        assert!(
            exit.contains(&format!(
                "last_output={access} last_error=[Warning] cert about to expire"
            )),
            "{exit}"
        );
        assert_eq!(msg, format!("process exited with code 3: {access}"));
    }

    #[tokio::test]
    async fn last_error_is_none_without_warnings() {
        let dir = tempfile::TempDir::new().unwrap();
        let (exit, _) = crash_exit_record(&dir, "echo 'started'\nexit 3\n").await;
        assert!(
            exit.ends_with("last_output=started last_error=none"),
            "{exit}"
        );
    }

    #[tokio::test]
    async fn last_error_matches_through_ansi() {
        let dir = tempfile::TempDir::new().unwrap();
        let (exit, _) = crash_exit_record(
            &dir,
            "printf '\\033[33m[Warning]\\033[0m tls retry\\n'\necho 'done'\nexit 3\n",
        )
        .await;
        assert!(exit.ends_with("last_error=[Warning] tls retry"), "{exit}");
    }

    fn stub_helper(dir: &std::path::Path, body: &str) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let calls = dir.join("calls");
        let helper = dir.join("netctl");
        std::fs::write(
            &helper,
            format!("#!/bin/sh\necho \"$1\" >> {}\n{body}", calls.display()),
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        (helper, calls)
    }

    fn xray_on_lo(helper: PathBuf) -> TunRuntime {
        TunRuntime {
            backend: BackendType::Xray,
            iface: "lo".into(),
            addr_v4: "172.19.0.1/30".into(),
            addr_v6: None,
            helper_path: helper,
            bypass_uid: None,
            capture_dns: false,
            strict: false,
        }
    }

    fn crashing_backend(dir: &std::path::Path, crashes: usize) -> String {
        let runs = dir.join("runs");
        format!(
            "echo run >> {runs}\nn=$(wc -l < {runs})\n[ $n -le {crashes} ] && exit 1\nexec sleep 30\n",
            runs = runs.display()
        )
    }

    fn read_lines(path: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    async fn wait_for_lines(path: &std::path::Path, n: usize) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let lines = read_lines(path);
            if lines.len() >= n || Instant::now() >= deadline {
                return lines;
            }
            sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn respawn_skips_preflight() {
        let dir = tempfile::TempDir::new().unwrap();
        let fresh = ProcessManager::new(
            dir.path().join("b"),
            dir.path().join("c"),
            dir.path().join("p"),
            None,
        );
        assert_eq!(fresh.restart_delay, CRASH_RESTART_DELAY);

        let checks = dir.path().join("checks");
        let script = format!(
            "{SINGBOX_VERSION_STUB}if [ \"$1\" = check ]; then echo check >> {}; exit 0; fi\n{}",
            checks.display(),
            crashing_backend(dir.path(), 1)
        );
        let mut mgr = manager_for(&dir, &script).with_backend(BackendType::SingBox);
        mgr.restart_delay = Duration::from_millis(50);

        mgr.start().await.unwrap();
        mgr.wait_and_handle_exit().await.unwrap();

        assert_eq!(mgr.state(), ProcessState::Running);
        assert_eq!(read_lines(&checks).len(), 1);
        assert_eq!(wait_for_lines(&dir.path().join("runs"), 2).await.len(), 2);
        mgr.shutdown().await;
    }

    #[tokio::test]
    async fn failed_respawn_is_retried() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, &crashing_backend(dir.path(), 1));
        mgr.restart_delay = Duration::from_millis(50);
        mgr.start().await.unwrap();

        let ups = dir.path().join("ups");
        let (helper, calls) = stub_helper(
            dir.path(),
            &format!(
                "[ \"$1\" = xray-up ] || exit 0\necho up >> {ups}\n[ $(wc -l < {ups}) -le 1 ] && exit 1\nexit 0\n",
                ups = ups.display()
            ),
        );
        mgr.tun = Some(xray_on_lo(helper));

        let mut rx = mgr.subscribe();
        mgr.wait_and_handle_exit().await.unwrap();

        assert_eq!(mgr.state(), ProcessState::Running);
        assert_eq!(
            drain_states(&mut rx),
            [ProcessState::Starting, ProcessState::Running]
        );
        assert_eq!(read_lines(&calls), ["xray-up", "xray-up"]);
        mgr.shutdown().await;
    }

    #[tokio::test]
    async fn respawn_budget_exhaustion_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, "exit 1\n");
        mgr.restart_delay = Duration::from_millis(50);
        mgr.start().await.unwrap();
        let (helper, calls) = stub_helper(dir.path(), "");
        mgr.tun = Some(xray_on_lo(helper));

        while mgr.state() == ProcessState::Running {
            mgr.wait_and_handle_exit().await.unwrap();
        }

        match mgr.state() {
            ProcessState::Error(msg) => assert!(msg.contains("3 crashes within 60s"), "{msg}"),
            other => panic!("expected Error state, got {other:?}"),
        }
        let calls = read_lines(&calls);
        assert_eq!(calls, ["xray-up", "xray-up"]);
        assert!(!calls.iter().any(|c| c == "xray-down"));
    }

    #[tokio::test]
    async fn stop_during_respawn_wait_reaches_stopped() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, &crashing_backend(dir.path(), 1));
        mgr.restart_delay = Duration::from_secs(5);
        mgr.start().await.unwrap();
        let (helper, calls) = stub_helper(dir.path(), "");
        mgr.tun = Some(xray_on_lo(helper));

        let waited =
            tokio::time::timeout(Duration::from_millis(300), mgr.wait_and_handle_exit()).await;
        assert!(waited.is_err(), "respawn should still be waiting");
        assert_eq!(mgr.state(), ProcessState::Starting);

        mgr.shutdown().await;
        assert_eq!(mgr.state(), ProcessState::Stopped);
        assert_eq!(read_lines(&calls), ["xray-down"]);
        assert_eq!(read_lines(&dir.path().join("runs")).len(), 1);
    }

    #[tokio::test]
    async fn crash_without_restart_keeps_routing_state() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, "exit 1\n");
        mgr.set_auto_restart(false);
        mgr.start().await.unwrap();
        let (helper, calls) = stub_helper(dir.path(), "");
        mgr.tun = Some(xray_on_lo(helper));

        mgr.wait_and_handle_exit().await.unwrap();

        assert!(matches!(mgr.state(), ProcessState::Error(_)));
        assert!(
            read_lines(&calls).is_empty(),
            "give-up must not call the helper"
        );
    }

    #[tokio::test]
    async fn tun_start_refuses_without_capability() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, "{}").unwrap();

        // /bin/sh exists but has no cap_net_admin, so the TUN gate must refuse
        // to start (whether getcap reports it missing or cannot be run).
        let mut mgr = ProcessManager::new(
            PathBuf::from("/bin/sh"),
            config,
            dir.path().join("backend.pid"),
            None,
        )
        .with_tun(Some(TunRuntime {
            backend: BackendType::Xray,
            iface: "tun0".into(),
            addr_v4: "172.19.0.1/30".into(),
            addr_v6: None,
            helper_path: dir.path().join("missing-netctl"),
            bypass_uid: None,
            capture_dns: false,
            strict: false,
        }));

        let result = mgr.start_with_connection(None).await;
        assert!(
            matches!(
                result,
                Err(ProcessError::TunCapabilityMissing(_))
                    | Err(ProcessError::TunCapabilityProbe(_))
            ),
            "expected a TUN capability error, got {result:?}"
        );
        assert!(matches!(mgr.state(), ProcessState::Error(_)));
        assert!(mgr.child.is_none(), "no backend should have been spawned");
    }

    #[tokio::test]
    async fn tun_start_refuses_when_helper_is_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, "{}").unwrap();

        let mut mgr = ProcessManager::new(
            PathBuf::from("/bin/sh"),
            config,
            dir.path().join("backend.pid"),
            None,
        )
        .with_tun(Some(TunRuntime {
            backend: BackendType::Xray,
            iface: "tun0".into(),
            addr_v4: "172.19.0.1/30".into(),
            addr_v6: None,
            helper_path: dir.path().join("missing-netctl"),
            bypass_uid: None,
            capture_dns: false,
            strict: false,
        }))
        .with_backend(BackendType::Xray)
        .with_log_file(Some(backend_log(dir.path())));

        let result = mgr.start_with_connection(None).await;
        assert!(
            matches!(result, Err(ProcessError::TunHelperMissing)),
            "expected TunHelperMissing, got {result:?}"
        );
        assert!(matches!(mgr.state(), ProcessState::Error(_)));
        assert!(mgr.child.is_none(), "no backend should have been spawned");
        let lines = wait_for_lines(&dir.path().join("backend.log"), 0).await;
        assert!(
            !lines.iter().any(|l| l.contains("session")),
            "a helper preflight failure must not write a session record: {lines:?}"
        );
    }

    #[tokio::test]
    async fn tun_start_refuses_when_helper_not_yet_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, "{}").unwrap();
        // A freshly granted relocated helper is group-executable only, which a
        // process that has not picked up the group yet cannot run: mode 0o644.
        let helper = dir.path().join("netctl");
        std::fs::write(&helper, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o644)).unwrap();

        let mut mgr = ProcessManager::new(
            PathBuf::from("/bin/sh"),
            config,
            dir.path().join("backend.pid"),
            None,
        )
        .with_tun(Some(xray_on_lo(helper)))
        .with_backend(BackendType::Xray)
        .with_log_file(Some(backend_log(dir.path())));

        let result = mgr.start_with_connection(None).await;
        assert!(
            matches!(result, Err(ProcessError::TunHelperRelogin)),
            "expected TunHelperRelogin, got {result:?}"
        );
        assert!(matches!(mgr.state(), ProcessState::Error(_)));
        assert!(mgr.child.is_none(), "no backend should have been spawned");
        let lines = wait_for_lines(&dir.path().join("backend.log"), 0).await;
        assert!(
            !lines.iter().any(|l| l.contains("session")),
            "a helper preflight failure must not write a session record: {lines:?}"
        );
    }

    #[tokio::test]
    async fn singbox_tun_start_skips_helper_gates() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, "{}").unwrap();

        // sing-box self-routes, so a missing helper must not block the start;
        // the backend capability gate still applies.
        let mut mgr = ProcessManager::new(
            PathBuf::from("/bin/sh"),
            config,
            dir.path().join("backend.pid"),
            None,
        )
        .with_tun(Some(TunRuntime {
            backend: BackendType::SingBox,
            iface: "tun0".into(),
            addr_v4: "172.19.0.1/30".into(),
            addr_v6: None,
            helper_path: dir.path().join("missing-netctl"),
            bypass_uid: None,
            capture_dns: false,
            strict: false,
        }))
        .with_backend(BackendType::SingBox);

        let result = mgr.start_with_connection(None).await;
        // Root skips the capability probe by design; what must hold on every
        // host is that the missing helper never gates a sing-box start.
        if crate::privilege::caps_check_needed(nix::unistd::geteuid().as_raw()) {
            assert!(
                matches!(
                    result,
                    Err(ProcessError::TunCapabilityMissing(_))
                        | Err(ProcessError::TunCapabilityProbe(_))
                ),
                "expected a backend capability error, got {result:?}"
            );
        } else {
            assert!(
                !matches!(
                    result,
                    Err(ProcessError::TunHelperMissing)
                        | Err(ProcessError::TunHelperRelogin)
                        | Err(ProcessError::TunHelperCapabilityMissing(_))
                ),
                "the missing helper must not gate a sing-box start, got {result:?}"
            );
        }
        assert!(mgr.child.is_none(), "no backend should have been spawned");
    }

    #[tokio::test]
    async fn stop_recovers_from_error_without_child() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, "{}").unwrap();
        // A missing binary fails start() before any child spawns, leaving the
        // manager in Error with no child — the state stop()/shutdown() must
        // recover instead of no-oping.
        let mut mgr = ProcessManager::new(
            dir.path().join("nonexistent-binary"),
            config,
            dir.path().join("backend.pid"),
            None,
        );

        let result = mgr.start().await;
        assert!(
            matches!(result, Err(ProcessError::BinaryNotFound(_))),
            "expected BinaryNotFound, got {result:?}"
        );
        assert!(matches!(mgr.state(), ProcessState::Error(_)));
        assert!(mgr.child.is_none());

        mgr.stop().await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Stopped);

        // stop() is idempotent on an already-Stopped manager.
        mgr.stop().await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Stopped);
    }

    fn drain_states(rx: &mut broadcast::Receiver<ProcessEvent>) -> Vec<ProcessState> {
        let mut states = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let ProcessEvent::StateChanged { to, .. } = event {
                states.push(to);
            }
        }
        states
    }

    #[tokio::test]
    async fn stop_from_starting_without_child_reaches_stopped() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(
            &dir,
            &format!("{SINGBOX_VERSION_STUB}[ \"$1\" = check ] && exec sleep 30\nexec sleep 30\n"),
        )
        .with_backend(BackendType::SingBox);

        let started =
            tokio::time::timeout(Duration::from_millis(300), mgr.start_with_connection(None)).await;
        assert!(
            started.is_err(),
            "start should still be checking the config"
        );
        assert_eq!(mgr.state(), ProcessState::Starting);
        assert!(mgr.child.is_none());

        let mut rx = mgr.subscribe();
        mgr.stop().await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Stopped);
        assert_eq!(
            drain_states(&mut rx),
            [ProcessState::Stopping, ProcessState::Stopped]
        );
    }

    #[tokio::test]
    async fn stop_from_running_without_child_reaches_stopped() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, "exec sleep 30\n");
        mgr.start().await.unwrap();
        let mut child = mgr.child.take().unwrap();
        child.kill().await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Running);

        let mut rx = mgr.subscribe();
        mgr.stop().await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Stopped);
        assert_eq!(
            drain_states(&mut rx),
            [ProcessState::Stopping, ProcessState::Stopped]
        );
    }

    #[tokio::test]
    async fn stop_from_error_releases_tun_state() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, "{}").unwrap();
        let mut mgr = ProcessManager::new(
            dir.path().join("nonexistent-binary"),
            config,
            dir.path().join("backend.pid"),
            None,
        );
        assert!(mgr.start().await.is_err());
        assert!(matches!(mgr.state(), ProcessState::Error(_)));

        let calls = dir.path().join("calls");
        let helper = dir.path().join("netctl");
        std::fs::write(
            &helper,
            format!("#!/bin/sh\necho \"$1\" >> {}\n", calls.display()),
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        mgr.tun = Some(TunRuntime {
            backend: BackendType::Xray,
            iface: "lo".into(),
            addr_v4: "172.19.0.1/30".into(),
            addr_v6: None,
            helper_path: helper,
            bypass_uid: None,
            capture_dns: false,
            strict: false,
        });

        let mut rx = mgr.subscribe();
        mgr.stop().await.unwrap();
        mgr.stop().await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Stopped);
        assert_eq!(drain_states(&mut rx), [ProcessState::Stopped]);
        assert_eq!(std::fs::read_to_string(&calls).unwrap(), "xray-down\n");
    }

    #[tokio::test]
    async fn teardown_failure_is_logged() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, "exec sleep 30\n");
        mgr.start().await.unwrap();

        let calls = dir.path().join("calls");
        let helper = dir.path().join("netctl");
        std::fs::write(
            &helper,
            format!("#!/bin/sh\necho \"$1\" >> {}\nexit 1\n", calls.display()),
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        mgr.tun = Some(TunRuntime {
            backend: BackendType::Xray,
            iface: "lo".into(),
            addr_v4: "172.19.0.1/30".into(),
            addr_v6: None,
            helper_path: helper,
            bypass_uid: None,
            capture_dns: false,
            strict: true,
        });

        mgr.stop().await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Stopped);
        assert_eq!(std::fs::read_to_string(&calls).unwrap(), "xray-down\n");
        let logged = mgr
            .log_buffer()
            .lock()
            .unwrap()
            .last_n(10)
            .iter()
            .any(|l| l.content.contains("xray-down failed"));
        assert!(logged, "expected the teardown failure in the log buffer");
    }

    #[test]
    fn parse_semver_triple_from_xray_version_output() {
        assert_eq!(
            parse_semver_triple("Xray 26.3.27 (Xray, Penetrates Everything.) cc66b68"),
            Some((26, 3, 27))
        );
        assert_eq!(parse_semver_triple("Xray 25.12.8 (...)"), Some((25, 12, 8)));
        assert_eq!(parse_semver_triple("no version here"), None);
        assert_eq!(
            parse_semver_triple("sing-box version 1.13.0\n\nEnvironment: go1.24.1 linux/amd64"),
            Some((1, 13, 0))
        );
    }

    #[test]
    fn xray_tun_minimum_version_comparison() {
        assert!((25, 12, 8) < XRAY_TUN_MIN_VERSION);
        assert!((26, 1, 12) < XRAY_TUN_MIN_VERSION);
        assert!((26, 1, 13) >= XRAY_TUN_MIN_VERSION);
        assert!((26, 3, 27) >= XRAY_TUN_MIN_VERSION);
    }

    #[test]
    fn singbox_minimum_version_comparison() {
        assert!((1, 11, 0) < SINGBOX_MIN_VERSION);
        assert!((1, 12, 4) < SINGBOX_MIN_VERSION);
        assert!((1, 13, 0) >= SINGBOX_MIN_VERSION);
        assert!((1, 14, 0) >= SINGBOX_MIN_VERSION);
    }

    fn contains_line(mgr: &ProcessManager, content: &str) -> bool {
        mgr.log_buffer()
            .lock()
            .unwrap()
            .last_n(10)
            .iter()
            .any(|l| l.content == content)
    }

    #[tokio::test]
    async fn singbox_start_fails_before_check_on_old_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let checked = dir.path().join("checked");
        let script = format!(
            "[ \"$1\" = version ] && {{ echo 'sing-box version 1.12.4'; exit 0; }}\n[ \"$1\" = check ] && {{ touch {}; exit 0; }}\nexec sleep 30\n",
            checked.display()
        );
        let mut mgr = manager_for(&dir, &script).with_backend(BackendType::SingBox);

        let result = mgr.start_with_connection(None).await;
        assert!(
            matches!(
                result,
                Err(ProcessError::BackendTooOld { backend: BackendType::SingBox, ref installed, ref required })
                    if installed == "1.12.4" && required == "1.13.0"
            ),
            "expected BackendTooOld, got {result:?}"
        );
        match mgr.state() {
            ProcessState::Error(msg) => {
                assert!(msg.contains("1.12.4") && msg.contains("1.13.0"), "{msg}")
            }
            other => panic!("expected Error state, got {other:?}"),
        }
        assert!(!checked.exists(), "config check must not run");
        assert!(mgr.child.is_none(), "no backend should have been spawned");
    }

    #[tokio::test]
    async fn singbox_start_proceeds_on_minimum_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(
            &dir,
            &format!("{SINGBOX_VERSION_STUB}[ \"$1\" = check ] && exit 0\nexec sleep 30\n"),
        )
        .with_backend(BackendType::SingBox);

        mgr.start_with_connection(None).await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Running);
        assert!(
            !mgr.log_buffer()
                .lock()
                .unwrap()
                .last_n(10)
                .iter()
                .any(|l| l.content.contains("minimum-version check skipped"))
        );
        mgr.stop().await.unwrap();
    }

    #[tokio::test]
    async fn singbox_start_warns_on_unreadable_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(
            &dir,
            "[ \"$1\" = version ] && exit 0\n[ \"$1\" = check ] && exit 0\nexec sleep 30\n",
        )
        .with_backend(BackendType::SingBox)
        .with_log_file(Some(backend_log(dir.path())));

        mgr.start_with_connection(None).await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Running);
        let warning = "warning: could not read sing-box version; minimum-version check skipped";
        assert!(
            contains_line(&mgr, warning),
            "expected the unreadable-version warning"
        );
        let lines = wait_for_lines(&dir.path().join("backend.log"), 2).await;
        assert!(
            lines
                .iter()
                .any(|l| l.ends_with(&format!("notice {warning}"))),
            "{lines:?}"
        );
        mgr.stop().await.unwrap();
    }

    #[tokio::test]
    async fn xray_tun_start_warns_on_unreadable_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut mgr = manager_for(&dir, "[ \"$1\" = version ] && exit 0\nexit 1\n")
            .with_tun(Some(xray_on_lo(dir.path().join("missing-netctl"))))
            .with_backend(BackendType::Xray)
            .with_host_probe(HostProbe {
                getcap: PathBuf::from("/bin/true"),
                helper: dir.path().join("missing-netctl"),
            });

        let result = mgr.start_with_connection(None).await;
        assert!(
            matches!(result, Err(ProcessError::TunHelperMissing)),
            "later TUN gates must still run, got {result:?}"
        );
        assert!(contains_line(
            &mgr,
            "warning: could not read xray version; minimum-version check skipped"
        ));
    }

    #[tokio::test]
    async fn xray_start_without_tun_skips_version_probe() {
        let dir = tempfile::TempDir::new().unwrap();
        let probed = dir.path().join("probed");
        let script = format!(
            "[ \"$1\" = version ] && {{ touch {}; exit 0; }}\n[ \"$2\" = -test ] && exit 0\nexec sleep 30\n",
            probed.display()
        );
        let mut mgr = manager_for(&dir, &script).with_backend(BackendType::Xray);

        mgr.start_with_connection(None).await.unwrap();
        assert_eq!(mgr.state(), ProcessState::Running);
        assert!(
            !probed.exists(),
            "a non-TUN xray start must not probe the version"
        );
        mgr.stop().await.unwrap();
    }

    #[test]
    fn tun_mount_unsupported_carries_manual_setcap_wording() {
        let unsupported = crate::privilege::PrivilegeError::Unsupported {
            path: PathBuf::from("/tmp/.mount_abc/usr/bin/xray"),
            caps: crate::privilege::BACKEND_CAPS.to_string(),
        };
        let text = ProcessError::TunMountUnsupported(unsupported.to_string()).to_string();
        assert!(text.contains("ignores file capabilities"), "{text}");
        assert!(text.contains("sudo setcap"), "{text}");
    }

    #[tokio::test]
    async fn tun_start_fails_fast_on_pre_tun_xray_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let binary = dir.path().join("fake-xray");
        std::fs::write(
            &binary,
            "#!/bin/sh\necho 'Xray 25.12.8 (Xray, Penetrates Everything.)'\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, "{}").unwrap();

        let rt = TunRuntime {
            backend: BackendType::Xray,
            iface: "tun-test".into(),
            addr_v4: "172.19.0.1/30".into(),
            addr_v6: None,
            helper_path: dir.path().join("missing-netctl"),
            bypass_uid: None,
            capture_dns: false,
            strict: false,
        };
        let mut mgr = ProcessManager::new(binary, config, dir.path().join("backend.pid"), None)
            .with_tun(Some(rt))
            .with_backend(BackendType::Xray);

        let result = mgr.start().await;
        assert!(
            matches!(
                result,
                Err(ProcessError::BackendTooOld { backend: BackendType::Xray, ref installed, ref required })
                    if installed == "25.12.8" && required == "26.1.13"
            ),
            "expected BackendTooOld, got {result:?}"
        );
        assert!(mgr.child.is_none());
    }

    #[tokio::test]
    async fn tun_start_warns_on_panic_affected_xray_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let binary = dir.path().join("fake-xray");
        std::fs::write(
            &binary,
            "#!/bin/sh\necho 'Xray 26.3.27 (Xray, Penetrates Everything.)'\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, "{}").unwrap();

        let rt = TunRuntime {
            backend: BackendType::Xray,
            iface: "tun-test".into(),
            addr_v4: "172.19.0.1/30".into(),
            addr_v6: None,
            helper_path: dir.path().join("missing-netctl"),
            bypass_uid: None,
            capture_dns: false,
            strict: false,
        };
        let mut mgr = ProcessManager::new(binary, config, dir.path().join("backend.pid"), None)
            .with_tun(Some(rt))
            .with_backend(BackendType::Xray);

        // The fake script has no capabilities, so the start still fails at the
        // CAP_NET_ADMIN gate — after the version warning has been recorded.
        let _ = mgr.start().await;

        let warned = mgr
            .log_buffer()
            .lock()
            .unwrap()
            .last_n(10)
            .iter()
            .any(|l| l.content.contains("Xray-core #6364"));
        assert!(warned, "expected a TUN panic advisory log line");
    }

    #[tokio::test]
    async fn ready_probe_accepts_live_backend() {
        let dir = tempfile::TempDir::new().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut mgr =
            manager_for(&dir, "exec sleep 30\n").with_ready_probe(listener.local_addr().unwrap());

        let started = Instant::now();
        mgr.start().await.unwrap();
        assert!(
            started.elapsed() >= STABILITY_WINDOW,
            "{:?}",
            started.elapsed()
        );
        assert_eq!(mgr.state(), ProcessState::Running);
        mgr.stop().await.unwrap();
    }

    #[tokio::test]
    async fn exit_before_ready_carries_last_output() {
        let dir = tempfile::TempDir::new().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut mgr = manager_for(&dir, "echo FATAL >&2\nexit 1\n")
            .with_ready_probe(listener.local_addr().unwrap());

        match mgr.start().await {
            Err(ProcessError::ExitedBeforeReady(msg)) => {
                assert!(msg.contains("code 1"), "{msg}");
                assert!(msg.contains("FATAL"), "{msg}");
            }
            other => panic!("expected ExitedBeforeReady, got {other:?}"),
        }
        assert!(matches!(mgr.state(), ProcessState::Error(_)));
        assert!(mgr.child.is_none());
    }

    #[tokio::test]
    async fn ready_timeout_stops_the_backend() {
        let dir = tempfile::TempDir::new().unwrap();
        let addr = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let mut mgr = manager_for(&dir, "exec sleep 30\n")
            .with_log_file(Some(backend_log(dir.path())))
            .with_ready_probe(addr);
        mgr.ready_timeout = Duration::from_millis(1500);
        assert!(mgr.ready_timeout > STABILITY_WINDOW);

        match mgr.start().await {
            Err(e @ ProcessError::ReadyTimeout { .. }) => {
                let text = e.to_string();
                assert!(text.contains(&addr.to_string()), "{text}");
                assert!(text.contains("1.5s"), "{text}");
            }
            other => panic!("expected ReadyTimeout, got {other:?}"),
        }
        assert!(mgr.child.is_none(), "the backend should have been reaped");

        let lines = read_lines(&dir.path().join("backend.log"));
        let exits: Vec<_> = lines.iter().filter(|l| l.contains(" exit ")).collect();
        assert_eq!(exits.len(), 1, "{lines:#?}");
        assert!(
            exits[0].contains("exit requested=false reason=start-failed"),
            "{}",
            exits[0]
        );
    }

    #[test]
    fn is_host_level_classifies_every_variant() {
        let cases: Vec<(ProcessError, bool)> = vec![
            (
                ProcessError::BinaryNotFound(PathBuf::from("/usr/bin/xray")),
                false,
            ),
            (
                ProcessError::ConfigMissing(PathBuf::from("/tmp/config.json")),
                false,
            ),
            (ProcessError::Spawn(std::io::Error::other("denied")), false),
            (ProcessError::Wait(std::io::Error::other("gone")), false),
            (
                ProcessError::Transition(TransitionError::Invalid {
                    from: ProcessState::Running,
                    to: ProcessState::Stopped,
                }),
                false,
            ),
            (
                ProcessError::TunCapabilityMissing(PathBuf::from("/usr/bin/xray")),
                true,
            ),
            (
                ProcessError::TunCapabilityProbe("getcap not found".into()),
                true,
            ),
            (
                ProcessError::TunMountUnsupported("ignores file capabilities".into()),
                true,
            ),
            (ProcessError::TunDeviceTimeout("tun-test".into()), false),
            (ProcessError::TunHelper("helper exited 1".into()), false),
            (ProcessError::TunHelperMissing, true),
            (ProcessError::TunHelperRelogin, true),
            (
                ProcessError::TunHelperCapabilityMissing(PathBuf::from(
                    "/usr/local/bin/v2ray-rs-netctl",
                )),
                true,
            ),
            (
                ProcessError::BackendTooOld {
                    backend: BackendType::Xray,
                    installed: "25.12.8".into(),
                    required: "26.1.13".into(),
                },
                true,
            ),
            (
                ProcessError::BackendTooOld {
                    backend: BackendType::SingBox,
                    installed: "1.12.4".into(),
                    required: "1.13.0".into(),
                },
                true,
            ),
            (ProcessError::ConfigCheck("bad dns config".into()), false),
            (
                ProcessError::ExitedBeforeReady("process exited with code 1: FATAL".into()),
                false,
            ),
            (
                ProcessError::ReadyTimeout {
                    addr: SocketAddr::from(([127, 0, 0, 1], 1080)),
                    timeout: READY_TIMEOUT,
                },
                false,
            ),
        ];
        for (error, host_level) in cases {
            assert_eq!(error.is_host_level(), host_level, "{error}");
        }
    }
}
