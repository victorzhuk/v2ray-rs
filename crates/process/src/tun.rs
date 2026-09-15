use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::time::sleep;

use v2ray_rs_core::models::BackendType;

/// How long to wait for an xray TUN device to appear after spawn before giving up.
pub const DEVICE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a route helper invocation may run before it is killed.
pub const HELPER_TIMEOUT: Duration = Duration::from_secs(10);
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);

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
    /// Install the fail-closed fallback routes, so traffic has nowhere to go
    /// while the tunnel device is missing. xray only.
    pub strict: bool,
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

/// `/proc/sys/net/ipv6` is absent when the kernel boots with `ipv6.disable=1`,
/// and is per network namespace.
pub fn host_has_ipv6() -> bool {
    Path::new("/proc/sys/net/ipv6").exists()
}

/// Captured output and outcome of one route helper invocation.
pub struct HelperRun {
    pub output: Vec<String>,
    pub result: Result<(), String>,
    pub timed_out: bool,
}

/// Runs `netctl xray-up` to assign the address and split routes.
pub(crate) async fn xray_up(rt: &TunRuntime) -> HelperRun {
    run_helper(&rt.helper_path, &xray_up_args(rt), HELPER_TIMEOUT).await
}

pub(crate) fn xray_up_args(rt: &TunRuntime) -> Vec<String> {
    let mut args = vec![
        "xray-up".to_string(),
        "--iface".to_string(),
        rt.iface.clone(),
        "--addr".to_string(),
        rt.addr_v4.clone(),
    ];
    if let Some(v6) = &rt.addr_v6 {
        args.extend(["--addr6".to_string(), v6.clone()]);
    }
    if let Some(uid) = rt.bypass_uid {
        args.extend(["--bypass-uid".to_string(), uid.to_string()]);
    }
    if rt.capture_dns {
        args.push("--capture-dns".to_string());
    }
    if rt.strict {
        args.push("--strict".to_string());
    }
    args
}

/// Runs `netctl xray-down` to remove the device (idempotent).
pub(crate) async fn xray_down(rt: &TunRuntime) -> HelperRun {
    let args = [
        "xray-down".to_string(),
        "--iface".to_string(),
        rt.iface.clone(),
    ];
    run_helper(&rt.helper_path, &args, HELPER_TIMEOUT).await
}

/// Runs the route helper with its output captured and its lifetime bounded.
/// `kill_on_drop` covers a caller that abandons the future mid-run; the
/// timeout covers a helper wedged in a netlink call.
pub async fn run_helper(helper: &Path, args: &[String], timeout: Duration) -> HelperRun {
    let verb = args.first().map_or("helper", String::as_str);
    let spawned = crate::spawn::spawn_with_etxtbsy_retry(|| {
        let mut cmd = Command::new(helper);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd
    })
    .await;
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            return HelperRun {
                output: Vec::new(),
                result: Err(format!("{verb}: {e}")),
                timed_out: false,
            };
        }
    };

    // Readers own the buffer outside the timed wait, so lines printed before a
    // timeout survive it.
    let output = Arc::new(Mutex::new(Vec::new()));
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|s| spawn_reader(s, Arc::clone(&output))),
        child
            .stderr
            .take()
            .map(|s| spawn_reader(s, Arc::clone(&output))),
    ]
    .into_iter()
    .flatten()
    .collect();

    let (result, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) if status.success() => (Ok(()), false),
        Ok(Ok(status)) => (Err(format!("{verb} exited with {status}")), false),
        Ok(Err(e)) => (Err(format!("{verb}: {e}")), false),
        Err(_) => {
            let _ = child.kill().await;
            (Err(format!("{verb} timed out after {timeout:?}")), true)
        }
    };

    for reader in readers {
        let abort = reader.abort_handle();
        if tokio::time::timeout(OUTPUT_DRAIN_TIMEOUT, reader)
            .await
            .is_err()
        {
            abort.abort();
        }
    }
    let output = std::mem::take(&mut *output.lock().unwrap_or_else(|e| e.into_inner()));
    HelperRun {
        output,
        result,
        timed_out,
    }
}

fn spawn_reader<R>(stream: R, sink: Arc<Mutex<Vec<String>>>) -> tokio::task::JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            sink.lock().unwrap_or_else(|e| e.into_inner()).push(line);
        }
    })
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
            strict: false,
        };
        assert!(mk(BackendType::Xray).needs_helper());
        assert!(!mk(BackendType::SingBox).needs_helper());
    }

    fn xray_rt(strict: bool) -> TunRuntime {
        TunRuntime {
            backend: BackendType::Xray,
            iface: "tun0".into(),
            addr_v4: "172.19.0.1/30".into(),
            addr_v6: Some("fdfe:dcba:9876::1/126".into()),
            helper_path: PathBuf::from("/nonexistent/v2ray-rs-netctl"),
            bypass_uid: Some(967),
            capture_dns: true,
            strict,
        }
    }

    #[test]
    fn xray_up_args_include_strict_when_set() {
        assert_eq!(
            xray_up_args(&xray_rt(true)),
            [
                "xray-up",
                "--iface",
                "tun0",
                "--addr",
                "172.19.0.1/30",
                "--addr6",
                "fdfe:dcba:9876::1/126",
                "--bypass-uid",
                "967",
                "--capture-dns",
                "--strict",
            ]
        );
    }

    #[test]
    fn xray_up_args_omit_strict_when_off() {
        let rt = TunRuntime {
            addr_v6: None,
            bypass_uid: None,
            capture_dns: false,
            ..xray_rt(false)
        };
        assert_eq!(
            xray_up_args(&rt),
            ["xray-up", "--iface", "tun0", "--addr", "172.19.0.1/30"]
        );
    }

    fn write_helper(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("netctl");
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn helper_killed_after_timeout() {
        let dir = tempfile::TempDir::new().unwrap();
        let pid_file = dir.path().join("pid");
        let helper = write_helper(
            dir.path(),
            &format!("echo $$ > {}\nexec sleep 30\n", pid_file.display()),
        );

        let started = Instant::now();
        let run = run_helper(
            &helper,
            &["xray-up".to_string()],
            Duration::from_millis(200),
        )
        .await;
        let err = run.result.expect_err("a hung helper must fail");
        assert!(err.contains("timed out"), "{err}");
        assert!(run.timed_out);
        assert!(started.elapsed() < Duration::from_secs(2));

        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
            Err(nix::errno::Errno::ESRCH)
        );
    }

    #[tokio::test]
    async fn helper_output_is_captured() {
        let dir = tempfile::TempDir::new().unwrap();
        let helper = write_helper(dir.path(), "echo up-ok\necho 'netctl: boom' >&2\nexit 1\n");

        let run = run_helper(&helper, &["xray-up".to_string()], HELPER_TIMEOUT).await;
        assert!(run.output.iter().any(|l| l == "up-ok"), "{:?}", run.output);
        assert!(
            run.output.iter().any(|l| l == "netctl: boom"),
            "{:?}",
            run.output
        );
        let err = run.result.expect_err("a non-zero exit must fail");
        assert!(err.contains("exit status"), "{err}");
        assert!(!run.timed_out);
    }
}
