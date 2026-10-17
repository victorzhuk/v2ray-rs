use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ipnet::IpNet;
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
    /// Destinations routed through the main table instead of the tunnel. xray
    /// only; sing-box excludes them in its own config.
    pub exclude_routes: Vec<String>,
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
pub fn helpers_stale(installed: &Path) -> bool {
    let Some(src) = bundled_helper_path() else {
        return false;
    };
    let (Ok(src_meta), Ok(dst_meta)) = (src.metadata(), installed.metadata()) else {
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

/// Why the route helper is or is not usable by the current process. Every
/// variant except `Ready` is terminal for startup; `NoExec` and
/// `ProbeFailed` are never repairable by re-granting or re-login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperAccess {
    Ready,
    Missing,
    EnrollmentRequired,
    ReloginRequired,
    Permissions,
    NoExec,
    ProbeFailed(String),
}

/// Classifies the helper at `path` for the calling account: executable now,
/// absent, wrong ownership/mode, on a noexec mount, missing enrollment, or a
/// session that predates enrollment. Blocking (NSS, `/proc/self/mounts`,
/// `access`); callers on an async runtime should use `spawn_blocking`.
pub fn helper_access(path: &Path) -> HelperAccess {
    use std::os::unix::fs::MetadataExt;

    // Only ever classify the exact stored path: a relative name would be
    // resolved by the kernel against the CWD, not by the launch path.
    if !path.is_absolute() {
        return HelperAccess::Missing;
    }
    let lmeta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return HelperAccess::Missing,
        Err(e) => {
            return HelperAccess::ProbeFailed(format!("helper metadata unreadable: {e}"));
        }
    };
    let file_type = lmeta.file_type();
    if file_type.is_symlink() || !file_type.is_file() {
        return HelperAccess::ProbeFailed("target is not a regular file".into());
    }
    let mode = lmeta.mode() & 0o7777;
    let owner = lmeta.uid();
    let file_gid = lmeta.gid();
    let canonical = match std::fs::canonicalize(path) {
        Ok(canonical) => canonical,
        Err(e) => {
            return HelperAccess::ProbeFailed(format!("helper path unresolvable: {e}"));
        }
    };
    let mount = mount_state(&canonical);
    let root = nix::unistd::getuid().is_root();
    let group = if root {
        Ok(None)
    } else {
        nix::unistd::Group::from_name(crate::privilege::HELPER_GROUP)
    };
    let permission = if root {
        // Root never consults the group: enrollment and re-login are
        // non-root concepts, and invalid root metadata is a probe failure.
        if owner == 0 && mode == 0o750 {
            PermissionVerdict::Correct
        } else {
            PermissionVerdict::Incorrect
        }
    } else {
        match &group {
            Err(_) => PermissionVerdict::GroupQueryFailed,
            Ok(None) if owner == 0 && mode == 0o750 => PermissionVerdict::Unenrolled,
            Ok(Some(group)) if owner == 0 && mode == 0o750 && file_gid == group.gid.as_raw() => {
                PermissionVerdict::Correct
            }
            Ok(_) => PermissionVerdict::Incorrect,
        }
    };
    let group_gid = group.as_ref().ok().and_then(|g| g.as_ref()).map(|g| g.gid);
    let live = if root {
        Query::No
    } else if let Some(gid) = group_gid {
        match nix::unistd::getgroups() {
            Ok(supplementary) => {
                Query::from(supplementary.contains(&gid) || nix::unistd::getgid() == gid)
            }
            Err(_) => Query::Failed,
        }
    } else {
        Query::No
    };
    let exec = match nix::unistd::access(path, nix::unistd::AccessFlags::X_OK) {
        Ok(()) => ExecState::Ok,
        Err(nix::errno::Errno::EACCES) => ExecState::Eacces,
        Err(nix::errno::Errno::ENOENT) => ExecState::Noent,
        Err(_) => ExecState::Other,
    };
    // Invariant: this condition mirrors the classifier's stale-session
    // branch, so the account NSS query only runs when its answer is used.
    let account = (!root
        && permission == PermissionVerdict::Correct
        && live == Query::No
        && exec == ExecState::Eacces)
        .then(|| account_membership(group_gid));
    classify_helper_access(HelperFacts {
        root,
        mount,
        permission,
        live,
        account,
        exec,
    })
}

/// Mount verdict for the canonical helper location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MountState {
    Exec,
    NoExec,
    Unknown,
}

/// Ownership/mode verdict, already interpreted for the calling uid: `Correct`
/// means root-owned, mode 0750 and, for non-root callers, owned by the
/// dedicated group's GID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionVerdict {
    Correct,
    Incorrect,
    GroupQueryFailed,
    Unenrolled,
}

/// Outcome of a membership or execution query. Errors stay errors; they never
/// read as negative membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Query {
    Yes,
    No,
    Failed,
}

impl From<bool> for Query {
    fn from(member: bool) -> Self {
        if member { Query::Yes } else { Query::No }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecState {
    Ok,
    Eacces,
    Noent,
    Other,
}

/// Everything the pure classifier needs; `account` stays `None` unless the
/// stale-session branch actually requires it.
struct HelperFacts {
    root: bool,
    mount: MountState,
    permission: PermissionVerdict,
    live: Query,
    account: Option<Query>,
    exec: ExecState,
}

const ROOT_EXEC_DENIED: &str =
    "root execution access denied. Check parent directories and security policy";
const ACTIVE_MEMBER_DENIED: &str = "execute permission denied despite active v2ray-rs group membership. Check parent directories and security policy";
const EXEC_WITHOUT_MEMBERSHIP: &str = "execution access does not match v2ray-rs group membership";

fn probe(detail: &str) -> HelperAccess {
    HelperAccess::ProbeFailed(detail.to_string())
}

fn classify_helper_access(facts: HelperFacts) -> HelperAccess {
    match facts.mount {
        MountState::NoExec => return HelperAccess::NoExec,
        MountState::Unknown => {
            return probe("could not read the mount options covering the helper");
        }
        MountState::Exec => {}
    }
    if facts.root {
        if facts.permission != PermissionVerdict::Correct {
            return probe("helper is not root-owned with mode 0750");
        }
        return match facts.exec {
            ExecState::Ok => HelperAccess::Ready,
            ExecState::Noent => HelperAccess::Missing,
            ExecState::Eacces => probe(ROOT_EXEC_DENIED),
            ExecState::Other => probe("execution check failed"),
        };
    }
    if facts.permission == PermissionVerdict::GroupQueryFailed {
        return probe("v2ray-rs group lookup failed");
    }
    if facts.permission == PermissionVerdict::Incorrect {
        return HelperAccess::Permissions;
    }
    if facts.permission == PermissionVerdict::Unenrolled {
        return HelperAccess::EnrollmentRequired;
    }
    if facts.exec == ExecState::Noent {
        return HelperAccess::Missing;
    }
    match (facts.live, facts.exec) {
        (Query::Yes, ExecState::Ok) => HelperAccess::Ready,
        (Query::Yes, ExecState::Eacces) => probe(ACTIVE_MEMBER_DENIED),
        (Query::No, ExecState::Ok) => probe(EXEC_WITHOUT_MEMBERSHIP),
        (Query::No, ExecState::Eacces) => match facts.account {
            None => probe("account membership unavailable"),
            Some(Query::Failed) => probe("account group lookup failed"),
            Some(Query::No) => HelperAccess::EnrollmentRequired,
            Some(Query::Yes) => HelperAccess::ReloginRequired,
        },
        (Query::Failed, _) => probe("live group query failed"),
        (Query::Yes | Query::No, ExecState::Noent) => HelperAccess::Missing,
        (_, ExecState::Other) => probe("execution check failed"),
    }
}

fn mount_state(canonical: &Path) -> MountState {
    let Ok(mounts) = std::fs::read_to_string("/proc/self/mounts") else {
        return MountState::Unknown;
    };
    match crate::privilege::mount_for_path(&mounts, canonical) {
        None => MountState::Unknown,
        Some(options) if options.split(',').any(|option| option == "noexec") => MountState::NoExec,
        Some(_) => MountState::Exec,
    }
}

fn account_membership(group_gid: Option<nix::unistd::Gid>) -> Query {
    let Some(group_gid) = group_gid else {
        return Query::Failed;
    };
    let account = match nix::unistd::User::from_uid(nix::unistd::getuid()) {
        Ok(Some(account)) => account,
        // An unresolvable account name is a failed lookup, not an answer.
        Ok(None) | Err(_) => return Query::Failed,
    };
    let name = match std::ffi::CString::new(account.name.as_str()) {
        Ok(name) => name,
        Err(_) => return Query::Failed,
    };
    match nix::unistd::getgrouplist(&name, account.gid) {
        Ok(gids) => Query::from(gids.contains(&group_gid)),
        Err(_) => Query::Failed,
    }
}

/// Prefixes of every attached network the tunnel must not swallow.
///
/// One snapshot per connection: Docker bridges, LANs and other attached
/// interfaces are excluded from `auto_route` capture so replies can return
/// through the interface they arrived on. Refreshed on the next connect, not
/// on interface changes mid-session.
pub fn connected_subnets(tun_iface: &str) -> std::io::Result<Vec<IpNet>> {
    let addresses = nix::ifaddrs::getifaddrs()?;
    connected_subnets_from(addresses, tun_iface)
}

fn connected_subnets_from(
    addresses: impl IntoIterator<Item = nix::ifaddrs::InterfaceAddress>,
    tun_iface: &str,
) -> std::io::Result<Vec<IpNet>> {
    use nix::net::if_::InterfaceFlags;
    use std::net::IpAddr;

    let mut nets: Vec<IpNet> = Vec::new();
    for ifaddr in addresses {
        let Some(address) = ifaddr.address else {
            continue;
        };
        if !ifaddr.flags.contains(InterfaceFlags::IFF_UP)
            || ifaddr
                .flags
                .intersects(InterfaceFlags::IFF_LOOPBACK | InterfaceFlags::IFF_POINTOPOINT)
            || ifaddr.interface_name == tun_iface
        {
            continue;
        }
        let (addr, family_v4) = match address.as_sockaddr_in() {
            Some(v4) => (IpAddr::V4(v4.ip()), true),
            None => match address.as_sockaddr_in6() {
                Some(v6) => (IpAddr::V6(v6.ip()), false),
                None => continue,
            },
        };
        let mask = ifaddr.netmask.as_ref().and_then(|mask| {
            if family_v4 {
                mask.as_sockaddr_in().map(|m| IpAddr::V4(m.ip()))
            } else {
                mask.as_sockaddr_in6().map(|m| IpAddr::V6(m.ip()))
            }
        });
        let Some(mask) = mask else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "interface {} reported no {} netmask",
                    ifaddr.interface_name,
                    family_str(addr),
                ),
            ));
        };
        let net = IpNet::with_netmask(addr, mask)
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("interface {}: {e}", ifaddr.interface_name),
                )
            })?
            .trunc();
        // Host routes and whole address spaces are useless as exclusions.
        let exhaustive = match net {
            IpNet::V4(n) => n.prefix_len() == 0 || n.prefix_len() == 32,
            IpNet::V6(n) => n.prefix_len() == 0 || n.prefix_len() == 128,
        };
        if exhaustive {
            continue;
        }
        nets.push(net);
    }
    nets.sort();
    nets.dedup();
    Ok(nets)
}

fn family_str(addr: std::net::IpAddr) -> &'static str {
    match addr {
        std::net::IpAddr::V4(_) => "IPv4",
        std::net::IpAddr::V6(_) => "IPv6",
    }
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
    for cidr in &rt.exclude_routes {
        args.extend(["--exclude".to_string(), cidr.clone()]);
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

    mod connected_subnets {
        use super::super::*;
        use nix::ifaddrs::InterfaceAddress;
        use nix::net::if_::InterfaceFlags;
        use nix::sys::socket::SockaddrStorage;
        use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4};

        fn storage_v4(ip: Ipv4Addr) -> SockaddrStorage {
            SockaddrStorage::from(SocketAddr::V4(SocketAddrV4::new(ip, 0)))
        }

        fn storage_v6(ip: Ipv6Addr) -> SockaddrStorage {
            SockaddrStorage::from(SocketAddr::V6(std::net::SocketAddrV6::new(ip, 0, 0, 0)))
        }

        fn record(
            name: &str,
            flags: InterfaceFlags,
            address: SockaddrStorage,
            netmask: Option<SockaddrStorage>,
        ) -> InterfaceAddress {
            InterfaceAddress {
                interface_name: name.to_string(),
                flags,
                address: Some(address),
                netmask,
                broadcast: None,
                destination: None,
            }
        }

        fn v4(name: &str, flags: InterfaceFlags, ip: Ipv4Addr, mask: Ipv4Addr) -> InterfaceAddress {
            record(name, flags, storage_v4(ip), Some(storage_v4(mask)))
        }

        fn v6(name: &str, flags: InterfaceFlags, ip: Ipv6Addr, mask: Ipv6Addr) -> InterfaceAddress {
            record(name, flags, storage_v6(ip), Some(storage_v6(mask)))
        }

        fn collect(records: Vec<InterfaceAddress>) -> std::io::Result<Vec<IpNet>> {
            connected_subnets_from(records, "tun0")
        }

        const UP: InterfaceFlags = InterfaceFlags::IFF_UP;

        #[test]
        fn collects_sorted_deduplicated_prefixes_of_both_families() {
            let nets = collect(vec![
                v4(
                    "docker0",
                    UP,
                    Ipv4Addr::new(172, 17, 0, 1),
                    Ipv4Addr::new(255, 255, 0, 0),
                ),
                v4(
                    "lan0",
                    UP,
                    Ipv4Addr::new(192, 168, 1, 10),
                    Ipv4Addr::new(255, 255, 255, 0),
                ),
                v6(
                    "lan0",
                    UP,
                    Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1),
                    Ipv6Addr::new(0xffff, 0xffff, 0, 0, 0, 0, 0, 0),
                ),
                v4(
                    "docker0",
                    UP,
                    Ipv4Addr::new(172, 17, 0, 2),
                    Ipv4Addr::new(255, 255, 0, 0),
                ),
            ])
            .unwrap();
            let expect: Vec<IpNet> = [
                "172.17.0.0/16".parse().unwrap(),
                "192.168.1.0/24".parse().unwrap(),
                "fd00::/32".parse::<IpNet>().unwrap(),
            ]
            .to_vec();
            assert_eq!(nets, expect);
        }

        #[test]
        fn skips_records_without_an_address() {
            let nets = collect(vec![InterfaceAddress {
                interface_name: "eth0".to_string(),
                flags: UP,
                address: None,
                netmask: None,
                broadcast: None,
                destination: None,
            }])
            .unwrap();
            assert!(nets.is_empty());
        }

        #[test]
        fn skips_down_loopback_point_to_point_and_tun_iface() {
            let nets = collect(vec![
                v4(
                    "down0",
                    InterfaceFlags::empty(),
                    Ipv4Addr::new(10, 0, 0, 1),
                    Ipv4Addr::new(255, 0, 0, 0),
                ),
                v4(
                    "lo",
                    UP | InterfaceFlags::IFF_LOOPBACK,
                    Ipv4Addr::new(127, 0, 0, 1),
                    Ipv4Addr::new(255, 0, 0, 0),
                ),
                v4(
                    "ppp0",
                    UP | InterfaceFlags::IFF_POINTOPOINT,
                    Ipv4Addr::new(10, 1, 0, 1),
                    Ipv4Addr::new(255, 255, 255, 252),
                ),
                v4(
                    "tun0",
                    UP,
                    Ipv4Addr::new(172, 19, 0, 1),
                    Ipv4Addr::new(255, 255, 255, 252),
                ),
            ])
            .unwrap();
            assert!(nets.is_empty());
        }

        #[test]
        fn skips_zero_host_and_exhaustive_prefixes() {
            let nets = collect(vec![
                v4("a", UP, Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::UNSPECIFIED),
                v4(
                    "b",
                    UP,
                    Ipv4Addr::new(10, 0, 0, 2),
                    Ipv4Addr::new(255, 255, 255, 255),
                ),
                v6("c", UP, Ipv6Addr::LOCALHOST, Ipv6Addr::UNSPECIFIED),
                v6(
                    "d",
                    UP,
                    Ipv6Addr::LOCALHOST,
                    Ipv6Addr::new(
                        0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff,
                    ),
                ),
            ])
            .unwrap();
            assert!(nets.is_empty());
        }

        #[test]
        fn retains_bridge_whose_prefix_contains_the_tun_subnet() {
            let nets = collect(vec![v4(
                "tunbr",
                UP,
                Ipv4Addr::new(172, 19, 0, 1),
                Ipv4Addr::new(255, 255, 0, 0),
            )])
            .unwrap();
            assert_eq!(nets, vec!["172.19.0.0/16".parse::<IpNet>().unwrap()]);
        }

        #[test]
        fn missing_netmask_is_invalid_data() {
            let err = collect(vec![record(
                "eth0",
                UP,
                storage_v4(Ipv4Addr::new(10, 0, 0, 1)),
                None,
            )])
            .unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
            assert!(err.to_string().contains("eth0"), "{err}");
        }

        #[test]
        fn non_contiguous_mask_is_invalid_data() {
            let err = collect(vec![v4(
                "eth0",
                UP,
                Ipv4Addr::new(10, 0, 0, 1),
                Ipv4Addr::new(255, 0, 255, 0),
            )])
            .unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
            assert!(err.to_string().contains("eth0"), "{err}");
        }

        #[test]
        fn cross_family_mask_is_invalid_data() {
            let err = collect(vec![record(
                "eth0",
                UP,
                storage_v4(Ipv4Addr::new(10, 0, 0, 1)),
                Some(storage_v6(Ipv6Addr::new(0xffff, 0, 0, 0, 0, 0, 0, 0))),
            )])
            .unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
            assert!(err.to_string().contains("eth0"), "{err}");
        }
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
            exclude_routes: Vec::new(),
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
            exclude_routes: vec!["10.15.12.100/32".into()],
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
                "--exclude",
                "10.15.12.100/32",
            ]
        );
    }

    #[test]
    fn xray_up_args_omit_strict_when_off() {
        let rt = TunRuntime {
            addr_v6: None,
            bypass_uid: None,
            capture_dns: false,
            exclude_routes: Vec::new(),
            ..xray_rt(false)
        };
        assert_eq!(
            xray_up_args(&rt),
            ["xray-up", "--iface", "tun0", "--addr", "172.19.0.1/30"]
        );
    }

    #[test]
    fn xray_up_args_pass_exclusions_in_order() {
        let rt = TunRuntime {
            addr_v6: None,
            bypass_uid: None,
            capture_dns: false,
            exclude_routes: vec!["10.15.12.100/32".into(), "91.230.107.224/32".into()],
            ..xray_rt(false)
        };
        assert_eq!(
            xray_up_args(&rt),
            [
                "xray-up",
                "--iface",
                "tun0",
                "--addr",
                "172.19.0.1/30",
                "--exclude",
                "10.15.12.100/32",
                "--exclude",
                "91.230.107.224/32",
            ]
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

    mod helper_access {
        use super::super::*;
        use nix::unistd::Gid;

        const ROOT_DENIED: &str =
            "root execution access denied. Check parent directories and security policy";
        const ACTIVE_DENIED: &str = "execute permission denied despite active v2ray-rs group membership. Check parent directories and security policy";
        const MISMATCH: &str = "execution access does not match v2ray-rs group membership";

        fn facts(
            root: bool,
            permission: PermissionVerdict,
            live: Query,
            account: Option<Query>,
            exec: ExecState,
        ) -> HelperFacts {
            HelperFacts {
                root,
                mount: MountState::Exec,
                permission,
                live,
                account,
                exec,
            }
        }

        fn grants(gid: u32) -> Vec<Gid> {
            vec![Gid::from_raw(gid)]
        }

        fn gid() -> Gid {
            Gid::from_raw(4242)
        }

        #[test]
        fn active_member_with_successful_execution_is_ready() {
            let f = facts(
                false,
                PermissionVerdict::Correct,
                Query::Yes,
                None,
                ExecState::Ok,
            );
            assert_eq!(classify_helper_access(f), HelperAccess::Ready);
        }

        #[test]
        fn active_member_denied_is_probe_failure() {
            let f = facts(
                false,
                PermissionVerdict::Correct,
                Query::Yes,
                None,
                ExecState::Eacces,
            );
            assert_eq!(
                classify_helper_access(f),
                HelperAccess::ProbeFailed(ACTIVE_DENIED.into())
            );
        }

        #[test]
        fn execution_without_live_membership_is_probe_failure() {
            let f = facts(
                false,
                PermissionVerdict::Correct,
                Query::No,
                None,
                ExecState::Ok,
            );
            assert_eq!(
                classify_helper_access(f),
                HelperAccess::ProbeFailed(MISMATCH.into())
            );
        }

        #[test]
        fn unenrolled_account_is_enrollment_required() {
            let f = facts(
                false,
                PermissionVerdict::Correct,
                Query::No,
                Some(Query::No),
                ExecState::Eacces,
            );
            assert_eq!(classify_helper_access(f), HelperAccess::EnrollmentRequired);
        }

        #[test]
        fn stale_session_with_account_membership_is_relogin_required() {
            let f = facts(
                false,
                PermissionVerdict::Correct,
                Query::No,
                Some(Query::Yes),
                ExecState::Eacces,
            );
            assert_eq!(classify_helper_access(f), HelperAccess::ReloginRequired);
        }

        #[test]
        fn failed_account_lookup_is_probe_failure() {
            let f = facts(
                false,
                PermissionVerdict::Correct,
                Query::No,
                Some(Query::Failed),
                ExecState::Eacces,
            );
            assert!(matches!(
                classify_helper_access(f),
                HelperAccess::ProbeFailed(_)
            ));
        }

        #[test]
        fn noexec_overrides_logout_and_permission_verdicts() {
            for (permission, live) in [
                (PermissionVerdict::Correct, Query::Yes),
                (PermissionVerdict::Incorrect, Query::No),
                (PermissionVerdict::Unenrolled, Query::No),
            ] {
                let mut f = facts(false, permission, live, None, ExecState::Eacces);
                f.mount = MountState::NoExec;
                assert_eq!(classify_helper_access(f), HelperAccess::NoExec);
            }
        }

        #[test]
        fn failed_live_group_query_is_probe_failure() {
            let f = facts(
                false,
                PermissionVerdict::Correct,
                Query::Failed,
                None,
                ExecState::Ok,
            );
            assert!(matches!(
                classify_helper_access(f),
                HelperAccess::ProbeFailed(_)
            ));
        }

        #[test]
        fn wrong_ownership_or_mode_is_permissions() {
            let f = facts(
                false,
                PermissionVerdict::Incorrect,
                Query::Yes,
                None,
                ExecState::Ok,
            );
            assert_eq!(classify_helper_access(f), HelperAccess::Permissions);
        }

        #[test]
        fn absent_group_on_correct_helper_is_enrollment_required() {
            let f = facts(
                false,
                PermissionVerdict::Unenrolled,
                Query::No,
                None,
                ExecState::Ok,
            );
            assert_eq!(classify_helper_access(f), HelperAccess::EnrollmentRequired);
        }

        #[test]
        fn root_with_correct_helper_and_execution_is_ready() {
            let f = facts(
                true,
                PermissionVerdict::Correct,
                Query::No,
                None,
                ExecState::Ok,
            );
            assert_eq!(classify_helper_access(f), HelperAccess::Ready);
        }

        #[test]
        fn root_execution_denied_is_probe_failure_without_account_lookup() {
            let f = facts(
                true,
                PermissionVerdict::Correct,
                Query::No,
                Some(Query::Yes),
                ExecState::Eacces,
            );
            assert_eq!(
                classify_helper_access(f),
                HelperAccess::ProbeFailed(ROOT_DENIED.into())
            );
        }

        #[test]
        fn invalid_root_metadata_is_probe_failure() {
            let f = facts(
                true,
                PermissionVerdict::Incorrect,
                Query::No,
                None,
                ExecState::Ok,
            );
            assert!(matches!(
                classify_helper_access(f),
                HelperAccess::ProbeFailed(_)
            ));
        }

        #[test]
        fn failed_group_lookup_is_probe_failure() {
            let f = facts(
                false,
                PermissionVerdict::GroupQueryFailed,
                Query::No,
                None,
                ExecState::Ok,
            );
            assert!(matches!(
                classify_helper_access(f),
                HelperAccess::ProbeFailed(_)
            ));
        }

        #[test]
        fn concurrent_disappearance_is_missing() {
            let f = facts(
                false,
                PermissionVerdict::Correct,
                Query::Yes,
                None,
                ExecState::Noent,
            );
            assert_eq!(classify_helper_access(f), HelperAccess::Missing);
            let root = facts(
                true,
                PermissionVerdict::Correct,
                Query::No,
                None,
                ExecState::Noent,
            );
            assert_eq!(classify_helper_access(root), HelperAccess::Missing);
        }

        // Supplementary-group shape: membership comes from getgroups(), not
        // only the primary GID.
        #[test]
        fn supplementary_membership_shapes_the_query() {
            assert!(Query::from(grants(gid().as_raw()).contains(&gid())) == Query::Yes);
            assert!(Query::from(grants(1).contains(&gid())) == Query::No);
        }
    }
}
