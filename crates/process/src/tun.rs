use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::time::sleep;

use v2ray_rs_core::models::BackendType;

/// How long to wait for an xray TUN device to appear after spawn before giving up.
pub const DEVICE_TIMEOUT: Duration = Duration::from_secs(10);

const HELPER_BIN: &str = "v2ray-rs-netctl";
const RUN_BIN: &str = "v2ray-rs-run";

/// Where the privileged grant installs helpers when the bundled copies sit on a
/// mount that ignores file capabilities, which is every AppImage. Root-owned and
/// off `$PATH`, so it can neither be tampered with by the invoking user nor
/// shadow a distribution package's `/usr/bin` copies.
pub const RELOCATE_DIR: &str = "/usr/local/lib/v2ray-rs";
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
    /// Steer port-53 traffic into the tunnel table. Without it a resolver on the
    /// local subnet is reached through the preserved LAN route, so the host
    /// resolves every name outside the tunnel.
    pub capture_dns: bool,
}

impl TunRuntime {
    /// xray creates the TUN device but does not program routes on Linux, so it
    /// needs the privileged helper. sing-box self-routes via `auto_route`.
    pub fn needs_helper(&self) -> bool {
        self.backend == BackendType::Xray
    }
}

/// Resolves the route helper for execution.
pub fn helper_path() -> PathBuf {
    resolve_bin(HELPER_BIN)
}

/// Resolves the SUID run wrapper for execution.
pub fn run_path() -> PathBuf {
    resolve_bin(RUN_BIN)
}

fn resolve_bin(name: &str) -> PathBuf {
    let bundled = bundled_bin(name);
    let caps_ok = bundled
        .as_deref()
        .and_then(Path::parent)
        .is_none_or(crate::privilege::file_caps_supported);
    let relocated = relocated_bin(name);
    pick_bin(bundled.as_deref(), caps_ok, relocated.as_deref(), name)
}

/// Chooses which copy of `name` to run. Pure so the ordering can be tested
/// without a filesystem: `bundled`/`relocated` are the paths that exist, and
/// `bundled_caps_ok` is whether the bundled copy's mount honours file
/// capabilities.
///
/// A capable bundled copy always wins, so a leftover relocated copy can never
/// shadow a correctly installed distribution package. The bundled copy is still
/// preferred over `$PATH` when it cannot hold capabilities, so the failure is a
/// visible permission error rather than a silent jump to an unrelated binary.
fn pick_bin(
    bundled: Option<&Path>,
    bundled_caps_ok: bool,
    relocated: Option<&Path>,
    name: &str,
) -> PathBuf {
    match (bundled, relocated) {
        (Some(b), _) if bundled_caps_ok => b.to_path_buf(),
        (_, Some(r)) => r.to_path_buf(),
        (Some(b), None) => b.to_path_buf(),
        (None, None) => PathBuf::from(name),
    }
}

/// Absolute path to `name` beside the running executable, or `None`. Never falls
/// back to a bare filename, so the result is safe to hand to a root-elevated
/// `setcap`/`chown`/`chmod`, which resolve a relative argument against the
/// process CWD rather than `$PATH`.
fn bundled_bin(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join(name);
    candidate.exists().then_some(candidate)
}

fn relocated_bin(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(RELOCATE_DIR).join(name);
    candidate.exists().then_some(candidate)
}

/// Absolute path to the route helper beside the running executable, if present.
/// This is the source a privileged grant copies from, never an execution target.
pub fn bundled_helper_path() -> Option<PathBuf> {
    bundled_bin(HELPER_BIN)
}

/// Absolute path to the SUID run wrapper beside the running executable, if present.
pub fn bundled_run_path() -> Option<PathBuf> {
    bundled_bin(RUN_BIN)
}

/// Whether the bundled helpers must be copied elsewhere before they can hold
/// capabilities. True inside an AppImage, whose squashfs the kernel always
/// mounts `nosuid`, and equally on a `nosuid` `/home` or a USB stick — the mount
/// is the fact that matters, so no AppImage detection is involved.
pub fn relocation_required() -> bool {
    bundled_helper_path()
        .as_deref()
        .and_then(Path::parent)
        .is_some_and(|dir| !crate::privilege::file_caps_supported(dir))
}

/// Absolute path the grant installs the route helper to.
pub fn relocated_helper_path() -> PathBuf {
    Path::new(RELOCATE_DIR).join(HELPER_BIN)
}

/// Whether the relocated helper predates the bundled one it was copied from.
///
/// Pure filesystem metadata, deliberately: this is polled from the preferences
/// page on every settings change, and the previous version spawned
/// `netctl --version` there, on the GTK thread, with no timeout. It also
/// compares the wrong thing -- a rebuilt helper at the same version reads as
/// current. The destination's mtime is the time of the last grant, so a source
/// newer than that is exactly the "needs re-granting" condition.
pub fn helpers_stale() -> bool {
    let Some(src) = bundled_helper_path() else {
        return false;
    };
    let (Ok(src_meta), Ok(dst_meta)) = (src.metadata(), relocated_helper_path().metadata()) else {
        return false;
    };
    stale_against(
        src_meta.len(),
        src_meta.modified().ok(),
        dst_meta.len(),
        dst_meta.modified().ok(),
    )
}

fn stale_against(
    src_len: u64,
    src_mtime: Option<std::time::SystemTime>,
    dst_len: u64,
    dst_mtime: Option<std::time::SystemTime>,
) -> bool {
    if src_len != dst_len {
        return true;
    }
    match (src_mtime, dst_mtime) {
        (Some(src), Some(dst)) => src > dst,
        // Unreadable timestamps are not evidence of staleness; a same-size
        // helper is far more likely current than not.
        _ => false,
    }
}

/// Whether the relocated helper exists but this process cannot execute it,
/// which is what a freshly granted install looks like until the user picks up
/// their new group membership.
pub fn helper_needs_relogin() -> bool {
    let helper = relocated_helper_path();
    helper.exists() && !can_execute(&helper)
}

fn can_execute(path: &Path) -> bool {
    nix::unistd::access(path, nix::unistd::AccessFlags::X_OK).is_ok()
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
    if rt.capture_dns {
        cmd.arg("--capture-dns");
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

    fn t(secs: u64) -> std::time::SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)
    }

    #[test]
    fn a_source_newer_than_the_last_grant_is_stale() {
        // The destination's mtime is when the grant ran.
        assert!(stale_against(100, Some(t(200)), 100, Some(t(100))));
        assert!(!stale_against(100, Some(t(100)), 100, Some(t(200))));
    }

    #[test]
    fn a_differently_sized_helper_is_stale_whatever_the_timestamps() {
        assert!(stale_against(101, Some(t(100)), 100, Some(t(200))));
    }

    #[test]
    fn unreadable_timestamps_do_not_imply_staleness() {
        assert!(!stale_against(100, None, 100, Some(t(200))));
        assert!(!stale_against(100, Some(t(200)), 100, None));
    }

    #[test]
    fn picks_the_bundled_copy_when_it_can_hold_capabilities() {
        // Development builds and distribution packages both land here.
        let bundled = Path::new("/usr/bin/v2ray-rs-netctl");
        assert_eq!(
            pick_bin(Some(bundled), true, None, HELPER_BIN),
            PathBuf::from("/usr/bin/v2ray-rs-netctl")
        );
    }

    #[test]
    fn a_relocated_copy_never_shadows_a_capable_bundled_one() {
        let bundled = Path::new("/usr/bin/v2ray-rs-netctl");
        let relocated = Path::new("/usr/local/lib/v2ray-rs/v2ray-rs-netctl");
        assert_eq!(
            pick_bin(Some(bundled), true, Some(relocated), HELPER_BIN),
            PathBuf::from("/usr/bin/v2ray-rs-netctl")
        );
    }

    #[test]
    fn picks_the_relocated_copy_once_a_grant_has_installed_it() {
        let bundled = Path::new("/tmp/.mount_abc/usr/bin/v2ray-rs-netctl");
        let relocated = Path::new("/usr/local/lib/v2ray-rs/v2ray-rs-netctl");
        assert_eq!(
            pick_bin(Some(bundled), false, Some(relocated), HELPER_BIN),
            PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-netctl")
        );
    }

    #[test]
    fn falls_back_to_the_capless_bundled_copy_before_a_grant() {
        // Running it fails with a permission error the interface can explain,
        // which beats silently reaching an unrelated binary on $PATH.
        let bundled = Path::new("/tmp/.mount_abc/usr/bin/v2ray-rs-netctl");
        assert_eq!(
            pick_bin(Some(bundled), false, None, HELPER_BIN),
            PathBuf::from("/tmp/.mount_abc/usr/bin/v2ray-rs-netctl")
        );
    }

    #[test]
    fn falls_back_to_path_when_nothing_is_installed() {
        assert_eq!(
            pick_bin(None, true, None, HELPER_BIN),
            PathBuf::from(HELPER_BIN)
        );
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
            capture_dns: false,
        };
        assert!(mk(BackendType::Xray).needs_helper());
        assert!(!mk(BackendType::SingBox).needs_helper());
    }
}
