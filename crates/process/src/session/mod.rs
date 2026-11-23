use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Semaphore, broadcast, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use v2ray_rs_core::ansi::strip_ansi;
use v2ray_rs_core::config::ConfigWriter;
use v2ray_rs_core::config::ResolvedNodeSet;
use v2ray_rs_core::config::effective_dns::{EffectiveDns, effective_dns};
use v2ray_rs_core::models::{
    AppSettings, BackendType, ConnectionMetadata, ConnectionNodeRef, DnsHijackMode, HostOverride,
    ManualNode, ProxyNode, RoutingRule, Subscription, resolve_effective_config,
    uses_imported_profile,
};
use v2ray_rs_core::persistence::{AppPaths, prepare_tun_session};
use v2ray_rs_core::resolve::{ConnectionCandidate, resolve_via_nodes};
use v2ray_rs_core::rotating_log::{DEFAULT_MAX_BYTES, RotatingFileWriter};

use crate::manager::ProcessAdvisory;
use crate::{
    AntiDpiRuntime, ProcessError, ProcessEvent, ProcessManager, ProcessState, StopReason,
    TunRuntime,
};

use self::backend_warning::{BackendWarnings, FailureClassification, classify_failure};
pub use self::health::Health;
use self::health::{DnsFailureWindow, HealthTracker};
mod backend_warning;
mod builders;
mod cleanup;
mod health;
pub mod probe;
mod supervisor;
#[cfg(test)]
mod tests;

pub use builders::v2ray_tun_warning;
use cleanup::strip_leading_timestamp;
pub use supervisor::spawn;

/// Everything the UI needs to hear from a running session.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    State(u64, ProcessState, Option<ConnectionMetadata>),
    LogLines(u64, Vec<String>),
    Toast(String),
    Health(u64, Health),
    DnsHealth(u64, bool),
    TunGrantRequired(u64),
    /// A manager's cleanup failed: the session keeps it parked and waits for
    /// a retry, reporting no terminal state until the rules are gone. The
    /// text names what may remain installed.
    CleanupPending(u64, String),
    /// The anti-DPI engine exited on its own after readiness: DPI
    /// interception is degraded while the proxy connection stays up. At most
    /// one notice per generation; a superseded session's notices carry a
    /// generation the app drops.
    EngineDegraded {
        generation: u64,
        detail: String,
    },
    NodePins(u64, Vec<HostOverride>),
    OfferSubscriptionRefresh {
        subscription_id: uuid::Uuid,
        message: String,
    },
}

/// Fan-in for [`SessionEvent`]s. Cheap to clone and safe to send from any
/// thread, so the UI bridge forwards into Relm4 without the supervisor knowing
/// anything about GTK.
#[derive(Clone)]
pub struct EventSender(mpsc::UnboundedSender<SessionEvent>);

impl EventSender {
    pub fn channel() -> (Self, mpsc::UnboundedReceiver<SessionEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self(tx), rx)
    }

    pub fn emit(&self, event: SessionEvent) {
        let _ = self.0.send(event);
    }
}

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
pub type TunLifecycle = Arc<Mutex<()>>;

pub struct SessionHandle {
    cmd_tx: mpsc::UnboundedSender<SessionCmd>,
}

pub struct SessionRequest {
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
    /// Addresses a node hostname last connected through, used when its lookup
    /// fails at connect time.
    pub last_good_pins: Vec<HostOverride>,
}

#[derive(Debug, Clone, Copy)]
pub struct HealthTiming {
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

enum SessionCmd {
    Stop(StopReason),
}

impl SessionHandle {
    /// Delivers a stop request to the session task. The command channel is
    /// unbounded, so the call never blocks and is safe on the GTK main thread
    /// and inside a Tokio worker. `Ok(())` means the task will observe the
    /// stop; `Err(SessionStopError::SessionGone)` means the session task has
    /// already exited and the request is undeliverable.
    pub fn stop(&self, reason: StopReason) -> Result<(), SessionStopError> {
        self.cmd_tx
            .send(SessionCmd::Stop(reason))
            .map_err(|_| SessionStopError::SessionGone)
    }
}

/// Why a stop request could not reach the session task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStopError {
    /// The session task has exited; there is nothing left to stop.
    SessionGone,
}
