use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// File capabilities the backend binary needs for TUN: create the device
/// (`cap_net_admin`), bind privileged ports (`cap_net_bind_service`), and open
/// raw sockets (`cap_net_raw`).
pub const BACKEND_CAPS: &str = "cap_net_admin,cap_net_bind_service,cap_net_raw+ep";

/// File capability the route helper needs: program addresses and routes.
pub const HELPER_CAPS: &str = "cap_net_admin+ep";

/// File capabilities the anti-DPI helper needs: raw sockets for re-injecting
/// the modified packets (`cap_net_raw`) plus `SO_MARK` on those sockets for
/// the loop guard (`cap_net_admin`) — the same pair the engine's `probe_caps`
/// demands.
pub const ANTIDPI_CAPS: &str = "cap_net_admin,cap_net_raw+ep";

/// The capability tokens a provisioned anti-DPI helper must hold.
const ANTIDPI_REQUIRED_CAPS: [&str; 2] = ["cap_net_admin", "cap_net_raw"];

/// System group a relocated route helper is restricted to. Same name the
/// distribution package's install hook creates, so the two agree on a host that
/// has both.
pub const HELPER_GROUP: &str = "v2ray-rs";

#[derive(Debug, thiserror::Error)]
pub enum PrivilegeError {
    /// A probe could not produce a verdict at all: `getcap` failed to spawn,
    /// or its `spawn_blocking` task was cancelled before finishing — the text
    /// of that `JoinError` is carried the same way.
    #[error("read capabilities of {0}: {1}")]
    Probe(PathBuf, String),
    /// Outcome of a `getcap` invocation that did not complete normally:
    /// the helper was missing, the timeout elapsed, or `getcap` exited
    /// non-zero. Wrapped so its Display is the exact user-visible line that
    /// the manager's `TunCapabilityProbe` passes through verbatim.
    #[error("{0}")]
    ProbeFailure(ProbeFailure),
    #[error(
        "{path} is on a filesystem that ignores file capabilities (e.g. mounted nosuid). \
         Grant manually after moving the binary, or run: {}",
        manual_command(.path, .caps)
    )]
    Unsupported { path: PathBuf, caps: String },
    #[error(
        "cannot install the TUN helper into {dest}: that filesystem ignores file \
         capabilities (mounted nosuid). Install the distribution package instead."
    )]
    UnsupportedDest { dest: PathBuf },
    #[error(
        "refusing TUN privilege grant for {path}: {detail}. Install the distribution \
         package or ask an administrator to install the binaries in a protected \
         root-owned directory"
    )]
    UnsafeGrantTarget { path: PathBuf, detail: String },
    #[error(
        "cannot grant TUN group membership to uid 0. Run the application as a non-root account"
    )]
    RootEnrollment,
    #[error("TUN privilege grant could not be verified for {path}: {detail}")]
    GrantVerification { path: PathBuf, detail: String },
    #[error("run {0}: {1}")]
    Spawn(&'static str, std::io::Error),
    /// The elevation itself failed; carries the captured stderr of the
    /// elevated script, or a placeholder when it produced no diagnostic.
    #[error("privilege grant was cancelled or failed: {0}")]
    GrantFailed(String),
}

/// Bound on `getcap` execution. `kill` on expiry — not `drop` — so a wedged
/// probe cannot keep its child alive past the start attempt.
pub(crate) const GETCAP_TIMEOUT: Duration = Duration::from_millis(5000);

/// How a `getcap` invocation failed to produce a capability verdict.
#[derive(Debug, PartialEq, Eq)]
pub enum ProbeFailure {
    /// The helper was still running at `GETCAP_TIMEOUT`.
    Timeout,
    /// The helper binary was not found on `PATH` (`ENOENT`); the user must
    /// install libcap before TUN can start.
    NotFound,
    /// The helper exists but could not be started (`EACCES`, `ETXTBSY`, ...).
    Spawn(String),
    /// The helper exited non-zero. The detail is the trimmed stderr, which
    /// may be empty.
    Exit(String),
}

impl fmt::Display for ProbeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&probe_error_text(self))
    }
}

/// Maps a probe failure to the exact line the user sees.
pub(crate) fn probe_error_text(failure: &ProbeFailure) -> String {
    match failure {
        ProbeFailure::Timeout => "could not verify TUN capabilities: getcap timed out".into(),
        ProbeFailure::NotFound => "getcap not found; install libcap to use TUN".into(),
        ProbeFailure::Spawn(err) => {
            format!("could not verify TUN capabilities: cannot run getcap: {err}")
        }
        ProbeFailure::Exit(detail) => {
            format!("could not verify TUN capabilities: getcap exited with {detail}")
        }
    }
}

/// True when the process must read file capabilities before starting TUN.
/// Root already holds every capability, so probing there only adds a failure
/// mode without revealing anything.
pub(crate) fn caps_check_needed(euid: u32) -> bool {
    euid != 0
}

mod grant;
mod probes;
#[cfg(test)]
mod tests;
mod verify;

pub(crate) use grant::validate_recovery_target;
pub use grant::{antidpi_grant_eligible, grant, manual_command};
pub(crate) use probes::{
    file_caps_supported, mount_for_path, probe_net_admin, target_caps_supported,
};
pub use probes::{has_antidpi_caps, has_net_admin};
