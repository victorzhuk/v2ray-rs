use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use thiserror::Error;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::antidpi::{ANTIDPI_READY_TIMEOUT, AntiDpiRuntime, EngineExit};
use crate::pid::PidFile;
use crate::spawn::{MAX_LOG_LINE_BYTES, read_bounded_line};
use crate::state::{ProcessEvent, ProcessState, StateManager, TransitionError};
use crate::tun::{HelperRun, TunRuntime};
use v2ray_rs_core::ansi::strip_ansi;
use v2ray_rs_core::log_buffer::{LogBuffer, LogLine, LogSource};
use v2ray_rs_core::models::{BackendType, ConnectionMetadata};
use v2ray_rs_core::rotating_log::{RotatingFileWriter, local_utc_offset};

mod lifecycle;
mod logs;
#[cfg(test)]
mod tests;
mod tun;

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
/// Upper bound on waiting for the anti-DPI engine to actually exit after the
/// stop signal: the engine's own SIGTERM budget plus slack for the wait.
const ANTIDPI_EXIT_WAIT: Duration = Duration::from_secs(10);
/// How often the engine monitor re-checks for a stop request or an exit.
const ANTIDPI_ENGINE_POLL: Duration = Duration::from_millis(100);
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
    HealthFailover,
    StartFailed,
}

impl StopReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            StopReason::UserStop => "user-stop",
            StopReason::NodeSwitch => "node-switch",
            StopReason::ApplyRestart => "apply-restart",
            StopReason::AppQuit => "app-quit",
            StopReason::HealthFailover => "health-failover",
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
    #[error("anti-DPI is unavailable: {0}")]
    AntiDpiMountUnsupported(String),
    #[error("TUN device {0} did not appear")]
    TunDeviceTimeout(String),
    #[error("TUN routing operation failed: {0}")]
    TunHelper(String),
    #[error("cleanup failed: TUN: {tun}; anti-DPI: {antidpi}")]
    CleanupFailed { tun: String, antidpi: String },
    #[error("anti-DPI helper failed: {0}")]
    AntiDpiHelper(String),
    #[error(
        "anti-DPI helper v2ray-rs-antidpi not found; install the v2ray-rs distribution package"
    )]
    AntiDpiHelperMissing,
    #[error("log out and back in to finish enabling the anti-DPI engine")]
    AntiDpiHelperRelogin,
    #[error(
        "anti-DPI helper {0} lacks cap_net_admin,cap_net_raw required for the anti-DPI engine; grant anti-DPI privileges first"
    )]
    AntiDpiHelperCapabilityMissing(PathBuf),
    #[error(
        "anti-DPI access requires membership in the v2ray-rs group. Grant anti-DPI privileges to enroll this account"
    )]
    AntiDpiHelperEnrollmentRequired,
    #[error(
        "anti-DPI helper {0} must be owned by root:v2ray-rs with mode 0750. Grant anti-DPI privileges to repair it"
    )]
    AntiDpiHelperPermissions(PathBuf),
    #[error(
        "anti-DPI helper {0} is on a noexec filesystem. Move or reinstall it on an executable filesystem"
    )]
    AntiDpiHelperNoExec(PathBuf),
    #[error("could not verify anti-DPI helper {path}: {detail}")]
    AntiDpiHelperAccessProbe { path: PathBuf, detail: String },
    #[error("could not verify anti-DPI capabilities: {0}")]
    AntiDpiCapabilityProbe(String),
    #[error(
        "anti-DPI engine exited before it was ready ({status}); last stderr: {}",
        stderr.as_deref().unwrap_or("<none>")
    )]
    AntiDpiStartupExited {
        status: String,
        stderr: Option<String>,
    },
    #[error("anti-DPI engine did not print its ready line on queue {queue} within {timeout:?}")]
    AntiDpiStartupTimeout { queue: u16, timeout: Duration },
    #[error("TUN recovery journal: {0}")]
    TunJournal(String),
    #[error("route helper v2ray-rs-netctl not found")]
    TunHelperMissing,
    #[error("log out and back in to finish enabling TUN")]
    TunHelperRelogin,
    #[error(
        "route helper {0} lacks CAP_NET_ADMIN required for TUN mode; grant TUN privileges first"
    )]
    TunHelperCapabilityMissing(PathBuf),
    #[error(
        "TUN access requires membership in the v2ray-rs group. Grant TUN privileges to enroll this account"
    )]
    TunHelperEnrollmentRequired,
    #[error(
        "route helper {0} must be owned by root:v2ray-rs with mode 0750. Grant TUN privileges to repair it"
    )]
    TunHelperPermissions(PathBuf),
    #[error(
        "route helper {0} is on a noexec filesystem. Move or reinstall it on an executable filesystem"
    )]
    TunHelperNoExec(PathBuf),
    #[error("could not verify route helper {path}: {detail}")]
    TunHelperAccessProbe { path: PathBuf, detail: String },
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
                | ProcessError::AntiDpiMountUnsupported(_)
                | ProcessError::TunHelperMissing
                | ProcessError::TunHelperRelogin
                | ProcessError::TunHelperCapabilityMissing(_)
                | ProcessError::TunHelperEnrollmentRequired
                | ProcessError::TunHelperPermissions(_)
                | ProcessError::TunHelperNoExec(_)
                | ProcessError::TunHelperAccessProbe { .. }
                | ProcessError::TunJournal(_)
                | ProcessError::CleanupFailed { .. }
                | ProcessError::AntiDpiHelper(_)
                | ProcessError::AntiDpiHelperMissing
                | ProcessError::AntiDpiHelperRelogin
                | ProcessError::AntiDpiHelperCapabilityMissing(_)
                | ProcessError::AntiDpiHelperEnrollmentRequired
                | ProcessError::AntiDpiHelperPermissions(_)
                | ProcessError::AntiDpiHelperNoExec(_)
                | ProcessError::AntiDpiHelperAccessProbe { .. }
                | ProcessError::AntiDpiCapabilityProbe(_)
                | ProcessError::AntiDpiStartupExited { .. }
                | ProcessError::AntiDpiStartupTimeout { .. }
                | ProcessError::BackendTooOld { .. }
        )
    }

    /// Maps the shared helper-access classification onto the startup and UI
    /// error surface. `Ready` needs no error.
    pub fn from_helper_access(path: &Path, access: crate::tun::HelperAccess) -> Option<Self> {
        use crate::tun::HelperAccess as Access;
        Some(match access {
            Access::Ready => return None,
            Access::Missing => ProcessError::TunHelperMissing,
            Access::EnrollmentRequired => ProcessError::TunHelperEnrollmentRequired,
            Access::ReloginRequired => ProcessError::TunHelperRelogin,
            Access::Permissions => ProcessError::TunHelperPermissions(path.to_path_buf()),
            Access::NoExec => ProcessError::TunHelperNoExec(path.to_path_buf()),
            Access::ProbeFailed(detail) => ProcessError::TunHelperAccessProbe {
                path: path.to_path_buf(),
                detail,
            },
        })
    }

    /// Maps the shared helper-access classification onto the anti-DPI start
    /// error surface, so a misprovisioned engine helper reports exactly what
    /// is wrong and which grant fixes it. `Ready` needs no error.
    pub fn from_antidpi_access(path: &Path, access: crate::tun::HelperAccess) -> Option<Self> {
        use crate::tun::HelperAccess as Access;
        Some(match access {
            Access::Ready => return None,
            Access::Missing => ProcessError::AntiDpiHelperMissing,
            Access::EnrollmentRequired => ProcessError::AntiDpiHelperEnrollmentRequired,
            Access::ReloginRequired => ProcessError::AntiDpiHelperRelogin,
            Access::Permissions => ProcessError::AntiDpiHelperPermissions(path.to_path_buf()),
            Access::NoExec => ProcessError::AntiDpiHelperNoExec(path.to_path_buf()),
            Access::ProbeFailed(detail) => ProcessError::AntiDpiHelperAccessProbe {
                path: path.to_path_buf(),
                detail,
            },
        })
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
    advisory_subs: Arc<Mutex<Vec<mpsc::UnboundedSender<ProcessAdvisory>>>>,
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
    antidpi: Option<AntiDpiRuntime>,
    antidpi_engine: Option<AntidpiEngine>,
    antidpi_rules_installed: bool,
    antidpi_session_id: Option<String>,
    antidpi_log_handles: Vec<tokio::task::JoinHandle<()>>,
    backend: Option<BackendType>,
    log_writer: Option<Arc<RotatingFileWriter>>,
    cached_version: Option<Option<String>>,
    stop_reason: StopReason,
    session_fields: Option<String>,
    session_extra: Vec<String>,
    #[cfg(any(test, feature = "test-utils"))]
    host_probe: Option<HostProbe>,
}

/// Live anti-DPI engine child, owned by its monitor task. The manager holds
/// the command channel and the exit record receiver; it never keeps the
/// child's numeric pid, so only the monitor signals and reaps the child.
struct AntidpiEngine {
    stop_tx: mpsc::UnboundedSender<()>,
    exit_rx: mpsc::UnboundedReceiver<EngineExit>,
}

/// An exit is confirmed only when the monitor delivered a record with a real
/// wait status. A wait failure or a missing record leaves the exit unknown.
fn exit_confirmed(exit: Option<&EngineExit>) -> bool {
    exit.is_some_and(|exit| !exit.wait_failed && exit.status.is_some())
}

/// Delivers one advisory to every live subscriber in order; a subscriber
/// whose receiver is gone is dropped here.
fn publish_advisory(
    subs: &Mutex<Vec<mpsc::UnboundedSender<ProcessAdvisory>>>,
    advisory: ProcessAdvisory,
) {
    let mut subs = subs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    subs.retain(|tx| tx.send(advisory.clone()).is_ok());
}

/// Mount gate for one privileged program: a `nosuid` filesystem ignores
/// file capabilities, so the `getcap` probes would report the program as
/// unprivileged no matter what was granted. Returns the `Unsupported`
/// text naming `path` and `caps`. A probe task that did not finish is not
/// evidence that the mount honors capabilities.
async fn unsupported_mount(path: &Path, caps: &'static str) -> Option<String> {
    let target = path.to_path_buf();
    let honors =
        tokio::task::spawn_blocking(move || crate::privilege::target_caps_supported(&target))
            .await
            .unwrap_or(false);
    (!honors).then(|| {
        crate::privilege::PrivilegeError::Unsupported {
            path: path.to_path_buf(),
            caps: caps.to_string(),
        }
        .to_string()
    })
}

/// Out-of-band notices that must reach the session even under log-broadcast
/// pressure. Carried on one dedicated unbounded channel per subscriber, never
/// on the lossy log broadcast.
#[derive(Clone, Debug)]
pub enum ProcessAdvisory {
    AntidpiDegraded { detail: String },
}

/// Pins the host facts the TUN preflight reads, so a test can reach the
/// capability verdict without depending on the mount table, the euid, or the
/// `getcap` and route helper installed on the machine.
#[cfg(any(test, feature = "test-utils"))]
#[derive(Debug, Clone)]
pub struct HostProbe {
    pub getcap: PathBuf,
    pub helper: PathBuf,
    /// Pinned helper verdict; `None` runs the real `helper_access` on the
    /// stored helper path. Pin `Ready` only when a test exercises a later gate.
    pub helper_access: Option<crate::tun::HelperAccess>,
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
        if self.antidpi_engine.take().is_some() {
            // Dropping the handle closes the stop channel: the monitor treats
            // that as a shutdown request, terminates and reaps the child. No
            // cached pid is signaled here — the monitor is the only signer.
            log::warn!("ProcessManager dropped with a live anti-DPI engine; its monitor stops it");
        }
        for handle in self.antidpi_log_handles.drain(..) {
            handle.abort();
        }
    }
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
            advisory_subs: Arc::new(Mutex::new(Vec::new())),
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
            antidpi: None,
            antidpi_engine: None,
            antidpi_rules_installed: false,
            antidpi_session_id: None,
            antidpi_log_handles: Vec::new(),
            backend: None,
            log_writer: None,
            cached_version: None,
            stop_reason: StopReason::UserStop,
            session_fields: None,
            session_extra: Vec::new(),
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

    /// Attaches the anti-DPI runtime so start/stop manage the engine's rule
    /// lifecycle and its child. The runtime is kept across crash respawns:
    /// a live engine child is reused, never double-spawned. One UUID is
    /// allocated per manager and reused on every install, engine run, and
    /// removal, so the helper's journal always matches this manager's rules.
    pub fn with_antidpi(mut self, antidpi: Option<AntiDpiRuntime>) -> Self {
        self.antidpi_session_id = antidpi.as_ref().map(|_| uuid::Uuid::new_v4().to_string());
        self.antidpi = antidpi;
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

    /// Writes one `dns` stream line per entry right after each session record.
    pub fn with_session_extra(mut self, lines: Vec<String>) -> Self {
        self.session_extra = lines;
        self
    }

    pub fn state(&self) -> ProcessState {
        self.state.state()
    }

    /// Reports whether a TUN runtime is attached, not whether routes are up.
    pub fn has_tun_runtime(&self) -> bool {
        self.tun.is_some()
    }

    /// Kernel identity this manager's TUN runtime claims:
    /// `(backend, iface, session_id)`. `None` without an attached runtime or
    /// a recorded journal, because such a manager cannot be matched against a
    /// replacement's adopted session.
    pub fn tun_identity(&self) -> Option<(BackendType, &str, &str)> {
        let rt = self.tun.as_ref()?;
        let journal = rt.journal.as_ref()?;
        Some((rt.backend, rt.iface.as_str(), journal.session_id.as_str()))
    }

    /// Disarms a superseded manager's TUN runtime without touching kernel
    /// state. A replacement candidate already adopted this manager's
    /// `(backend, iface, session_id)`, so kernel ownership passed to it and
    /// the teardown obligation here is void. Shutdown afterwards releases
    /// only this manager's child process and log readers.
    pub fn abandon_superseded_tun(&mut self) {
        self.tun = None;
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProcessEvent> {
        self.state.subscribe()
    }

    pub fn subscribe_logs(&self) -> broadcast::Receiver<ProcessEvent> {
        self.state.subscribe_logs()
    }

    /// Subscribes to the advisory channel: the lossless counterpart to the
    /// log broadcast for notices a lagging log consumer must never drop.
    pub fn subscribe_advisories(&self) -> mpsc::UnboundedReceiver<ProcessAdvisory> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.advisory_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(tx);
        rx
    }

    pub fn log_buffer(&self) -> &Arc<Mutex<LogBuffer>> {
        &self.log_buffer
    }

    pub fn set_auto_restart(&mut self, enabled: bool) {
        self.auto_restart = enabled;
    }
}

fn write_stream_line(writer: &Option<Arc<RotatingFileWriter>>, stream: &str, line: &str) {
    if let Some(writer) = writer {
        writer.append_line(stream, line);
    }
}
