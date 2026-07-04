use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::time::sleep;

use v2ray_rs_core::models::BackendType;

/// How long to wait for an xray TUN device to appear after spawn before giving up.
pub const DEVICE_TIMEOUT: Duration = Duration::from_secs(10);

const HELPER_BIN: &str = "v2ray-rs-netctl";
const RUN_BIN: &str = "v2ray-rs-run";
/// Name of the dedicated unprivileged system user used for xray TUN bypass.
pub const BYPASS_USER: &str = "v2ray-rs-bypass";

/// Everything the process manager needs to drive TUN mode for a connection.
#[derive(Debug, Clone)]
pub struct TunRuntime {
    pub backend: BackendType,
    pub iface: String,
    pub addr_v4: String,
    pub addr_v6: Option<String>,
    pub helper_path: PathBuf,
    pub bypass_uid: Option<u32>,
}

impl TunRuntime {
    /// xray creates the TUN device but does not program routes on Linux, so it
    /// needs the privileged helper. sing-box self-routes via `auto_route`.
    pub fn needs_helper(&self) -> bool {
        self.backend == BackendType::Xray
    }
}

/// Resolves the route helper: a sibling of the running executable (dev /
/// `cargo run`), otherwise the bare name resolved via `$PATH` at exec time.
pub fn helper_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join(HELPER_BIN);
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from(HELPER_BIN)
}

/// Resolves the SUID run wrapper: a sibling of the running executable (dev /
/// `cargo run`), otherwise the bare name resolved via `$PATH` at exec time.
pub fn run_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join(RUN_BIN);
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from(RUN_BIN)
}

/// Resolves a helper binary to an absolute path beside the running executable,
/// or `None` if it isn't there. Unlike [`helper_path`]/[`run_path`] it never
/// falls back to a bare filename, so the result is safe to hand to a
/// root-elevated `setcap`/`chown`/`chmod` (which resolve a relative argument
/// against the process CWD, not `$PATH`).
fn sibling_bin(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join(name);
    candidate.exists().then_some(candidate)
}

/// Absolute path to the route helper beside the running executable, if present.
pub fn helper_path_strict() -> Option<PathBuf> {
    sibling_bin(HELPER_BIN)
}

/// Absolute path to the SUID run wrapper beside the running executable, if present.
pub fn run_path_strict() -> Option<PathBuf> {
    sibling_bin(RUN_BIN)
}

/// Polls `/sys/class/net/<iface>` until the device exists or the timeout elapses.
pub async fn wait_for_device(iface: &str, timeout: Duration) -> bool {
    let path = device_path(iface);
    let deadline = Instant::now() + timeout;
    loop {
        if Path::new(&path).exists() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        sleep(Duration::from_millis(100)).await;
    }
}

fn device_path(iface: &str) -> String {
    format!("/sys/class/net/{iface}")
}

/// Runs `netctl xray-up` to assign the address and split routes.
pub async fn xray_up(rt: &TunRuntime) -> std::io::Result<bool> {
    let mut cmd = Command::new(&rt.helper_path);
    cmd.arg("xray-up")
        .arg("--iface")
        .arg(&rt.iface)
        .arg("--addr")
        .arg(&rt.addr_v4);
    if let Some(v6) = &rt.addr_v6 {
        cmd.arg("--addr6").arg(v6);
    }
    if let Some(uid) = rt.bypass_uid {
        cmd.arg("--bypass-uid").arg(uid.to_string());
    }
    Ok(cmd.status().await?.success())
}

/// Runs `netctl xray-down` to remove the device (idempotent).
pub async fn xray_down(rt: &TunRuntime) -> std::io::Result<bool> {
    Ok(Command::new(&rt.helper_path)
        .arg("xray-down")
        .arg("--iface")
        .arg(&rt.iface)
        .status()
        .await?
        .success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wait_for_existing_device_returns_immediately() {
        // `lo` always exists on Linux.
        assert!(wait_for_device("lo", Duration::from_secs(1)).await);
    }

    #[tokio::test]
    async fn wait_for_missing_device_times_out() {
        assert!(!wait_for_device("nonexistent-tun-xyz", Duration::from_millis(250)).await);
    }

    #[test]
    fn xray_needs_helper_singbox_does_not() {
        let mk = |backend| TunRuntime {
            backend,
            iface: "tun0".into(),
            addr_v4: "172.19.0.1/30".into(),
            addr_v6: None,
            helper_path: PathBuf::from("v2ray-rs-netctl"),
            bypass_uid: None,
        };
        assert!(mk(BackendType::Xray).needs_helper());
        assert!(!mk(BackendType::SingBox).needs_helper());
    }
}
