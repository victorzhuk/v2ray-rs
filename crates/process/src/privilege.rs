use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
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

/// Reports whether the binary at `path` already holds `cap_net_admin` in its
/// file capabilities. Bounded by `GETCAP_TIMEOUT`; a wedged probe is killed
/// (not abandoned) so it cannot outlive the start attempt.
pub fn has_net_admin(path: &Path) -> Result<bool, PrivilegeError> {
    probe_net_admin(Path::new("getcap"), path)
}

pub(crate) fn probe_net_admin(getcap: &Path, path: &Path) -> Result<bool, PrivilegeError> {
    probe_net_admin_within(getcap, path, GETCAP_TIMEOUT)
}

fn probe_net_admin_within(
    getcap: &Path,
    path: &Path,
    timeout: Duration,
) -> Result<bool, PrivilegeError> {
    let mut child = Command::new(getcap)
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PrivilegeError::ProbeFailure(ProbeFailure::NotFound)
            } else {
                PrivilegeError::ProbeFailure(ProbeFailure::Spawn(e.to_string()))
            }
        })?;

    let deadline = Instant::now() + timeout;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(PrivilegeError::ProbeFailure(ProbeFailure::Timeout));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                return Err(PrivilegeError::ProbeFailure(ProbeFailure::Exit(
                    e.to_string(),
                )));
            }
        }
    };

    let stdout = collect(&stdout, deadline)?;
    let stderr = collect(&stderr, deadline)?;
    if !status.success() {
        return Err(PrivilegeError::ProbeFailure(ProbeFailure::Exit(
            String::from_utf8_lossy(&stderr).trim().to_string(),
        )));
    }

    Ok(getcap_has_cap(
        &String::from_utf8_lossy(&stdout),
        "cap_net_admin",
    ))
}

/// Reads a pipe to EOF on its own thread. An exited `getcap` does not close
/// the pipe while a descendant still holds the inherited descriptor, so the
/// read must never run on the caller's deadline-bound path. A reader left
/// blocked that way ends once the last holder closes the descriptor.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    match pipe {
        Some(mut pipe) => {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = pipe.read_to_end(&mut buf);
                let _ = tx.send(buf);
            });
        }
        None => {
            let _ = tx.send(Vec::new());
        }
    }
    rx
}

fn collect(output: &mpsc::Receiver<Vec<u8>>, deadline: Instant) -> Result<Vec<u8>, PrivilegeError> {
    match output.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(buf) => Ok(buf),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err(PrivilegeError::ProbeFailure(ProbeFailure::Timeout))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Ok(Vec::new()),
    }
}

/// What a single `pkexec` elevation has to do, before target validation.
///
/// `InPlace` grants capabilities where the binaries already live. `Relocate`
/// first copies the route helper somewhere it can hold them, which is the only
/// option when the bundled copy sits on a `nosuid` mount.
#[derive(Debug, PartialEq, Eq)]
enum Selection {
    InPlace {
        helper: PathBuf,
        wrapper: Option<PathBuf>,
    },
    Relocate {
        dest_dir: PathBuf,
        src: PathBuf,
        dst: PathBuf,
    },
}

/// A validated mutation target: its canonical path and decimal `device:inode`
/// identity, re-checked inside the elevated script so nothing can be swapped
/// across the polkit prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetId {
    path: PathBuf,
    identity: String,
}

/// A fully validated elevation: every path is canonical and carries the
/// identity the elevated script must observe before mutating anything.
#[derive(Debug, PartialEq, Eq)]
enum GrantPlan {
    InPlace {
        backend: TargetId,
        helper: TargetId,
        wrapper: Option<TargetId>,
        user: String,
        uid: u32,
    },
    Relocate {
        backend: TargetId,
        dest_dir: PathBuf,
        src: PathBuf,
        dst: PathBuf,
        /// The destination helper's recorded `device:inode`, or `absent`.
        prior_dst: String,
        user: String,
        uid: u32,
    },
}

/// Refuses a root caller outright: enrolling uid 0 in the helper group would
/// grant nothing and hide a misconfigured launch.
fn ensure_not_root(uid: u32, euid: u32) -> Result<(), PrivilegeError> {
    if uid == 0 || euid == 0 {
        Err(PrivilegeError::RootEnrollment)
    } else {
        Ok(())
    }
}

/// Resolves the invoking account so the elevated script can pin the exact UID
/// it enrolls. An NSS failure is an error, never a guessed name.
fn invoking_account(at: &Path) -> Result<(u32, String), PrivilegeError> {
    let uid = nix::unistd::getuid();
    let user = nix::unistd::User::from_uid(uid)
        .map_err(|e| {
            PrivilegeError::Probe(
                at.to_path_buf(),
                format!("cannot resolve the invoking account: {e}"),
            )
        })?
        .ok_or_else(|| {
            PrivilegeError::Probe(
                at.to_path_buf(),
                "cannot resolve the invoking account".into(),
            )
        })?;
    Ok((u32::from(uid), user.name))
}

/// Grants the required capabilities via a single `pkexec` elevation. Blocks
/// until the polkit dialog completes.
pub fn grant(backend: &Path, helper: &Path) -> Result<(), PrivilegeError> {
    ensure_not_root(
        u32::from(nix::unistd::getuid()),
        u32::from(nix::unistd::geteuid()),
    )?;
    let (uid, user) = invoking_account(backend)?;
    let selection = plan_grant(helper)?;

    // setcap and chmod u+s exit 0 on a nosuid mount, so flag the offending path
    // and its remedy before prompting for elevation rather than after.
    for (path, caps) in preflight_targets(backend, &selection) {
        if !file_caps_supported(path) {
            return Err(match &selection {
                Selection::Relocate { dest_dir, .. } if path == dest_dir => {
                    PrivilegeError::UnsupportedDest {
                        dest: dest_dir.clone(),
                    }
                }
                _ => PrivilegeError::Unsupported {
                    path: path.to_path_buf(),
                    caps: caps.to_string(),
                },
            });
        }
    }

    let backend_id = validate_mutation_target(backend)?;
    match selection {
        Selection::InPlace { helper, wrapper } => {
            let helper_id = validate_mutation_target(&helper)?;
            let wrapper_id = wrapper
                .as_deref()
                .map(validate_mutation_target)
                .transpose()?;
            // A mode repair can enlarge an existing ACL mask and activate
            // dormant named permissions, so both files are inspected before
            // any elevation offers to change them.
            acl_allows_mode_change(&helper_id.path)?;
            if let Some(w) = &wrapper_id {
                acl_allows_mode_change(&w.path)?;
            }
            let plan = GrantPlan::InPlace {
                backend: backend_id,
                helper: helper_id,
                wrapper: wrapper_id,
                user,
                uid,
            };
            let output = run_elevation(&plan)?;
            if !output.status.success() {
                return Err(grant_failed(&output));
            }
            let GrantPlan::InPlace {
                backend,
                helper,
                wrapper,
                user,
                ..
            } = &plan
            else {
                unreachable!("plan just constructed as InPlace")
            };
            verify_grant_target(&helper.path, &helper.identity, 0o750)?;
            verify_caps(&helper.path)?;
            verify_enrollment(user, &helper.path)?;
            if let Some(w) = wrapper {
                verify_grant_target(&w.path, &w.identity, 0o4750)?;
            }
            verify_identity(backend)?;
            verify_caps(&backend.path)?;
        }
        Selection::Relocate { dest_dir, src, dst } => {
            validate_destination(&dest_dir)?;
            let prior_dst = prior_identity(&dst)?;
            let plan = GrantPlan::Relocate {
                backend: backend_id,
                dest_dir,
                src,
                dst,
                prior_dst,
                user,
                uid,
            };
            let output = run_elevation(&plan)?;
            if !output.status.success() {
                return Err(grant_failed(&output));
            }
            let GrantPlan::Relocate {
                backend, dst, user, ..
            } = &plan
            else {
                unreachable!("plan just constructed as Relocate")
            };
            let stdout = String::from_utf8_lossy(&output.stdout);
            let installed = parse_helper_identity(&stdout).ok_or_else(|| {
                PrivilegeError::GrantVerification {
                    path: dst.clone(),
                    detail: "elevation output is not a single helper-identity record".into(),
                }
            })?;
            let identity = format!("{}:{}", installed.0, installed.1);
            verify_grant_target(dst, &identity, 0o750)?;
            verify_caps(dst)?;
            verify_enrollment(user, dst)?;
            verify_identity(backend)?;
            verify_caps(&backend.path)?;
        }
    }
    Ok(())
}

/// Runs the single `pkexec` elevation and captures its output so relocation
/// can verify the identity record the script emits.
///
/// A relocating plan pipes the helper in on stdin rather than having root copy
/// it: an AppImage's squashfs is a FUSE mount with no `allow_other`, and
/// `fuse_allow_current_process()` grants root no exemption, so a `cp` under
/// `pkexec` would fail with EACCES. We are the mounting uid, so we can read it;
/// root only ever writes the destination. Opening the source here also closes
/// the window between validating the path and reading it.
fn run_elevation(plan: &GrantPlan) -> Result<std::process::Output, PrivilegeError> {
    let mut cmd = Command::new("pkexec");
    cmd.args(grant_argv(plan));

    let GrantPlan::Relocate { src, .. } = plan else {
        return cmd.output().map_err(|e| PrivilegeError::Spawn("pkexec", e));
    };

    let payload = std::fs::File::open(src)
        .map_err(|e| PrivilegeError::Probe(src.clone(), format!("cannot read: {e}")))?;
    cmd.stdin(payload);
    cmd.output().map_err(|e| PrivilegeError::Spawn("pkexec", e))
}

/// Fails the grant with the elevation's captured stderr; an empty stream gets
/// a placeholder rather than a bare refusal.
fn grant_failed(output: &std::process::Output) -> PrivilegeError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    let detail = if detail.is_empty() {
        "no diagnostic output".to_string()
    } else {
        detail.to_string()
    };
    PrivilegeError::GrantFailed(detail)
}

/// Decides between granting in place and relocating, resolving the paths the
/// elevation will touch.
///
/// The copy source comes from `/proc/self/exe` only. Taking it from `$APPDIR`
/// would let anything able to set that variable — a rewritten desktop entry,
/// for instance — choose what root installs.
fn plan_grant(helper: &Path) -> Result<Selection, PrivilegeError> {
    // A bare or relative path would be resolved against the elevated process's
    // CWD, so a planted file could be given capabilities; refuse anything not
    // found beside our own executable.
    let bundled = crate::tun::bundled_helper_path().ok_or_else(|| {
        PrivilegeError::Probe(
            helper.to_path_buf(),
            "route helper not found beside the executable".into(),
        )
    })?;

    if !crate::tun::relocation_required() {
        let selected = if helper.is_absolute() {
            helper.to_path_buf()
        } else {
            bundled.clone()
        };
        // Exact filename and parent only: the packaged copy, the bundled copy,
        // or the fixed relocated install. Anything else a caller hands in is
        // refused before elevation.
        let acceptable = paths_equivalent(&selected, &bundled)
            || paths_equivalent(&selected, &crate::tun::relocated_helper_path())
            || (selected.is_absolute()
                && selected
                    .file_name()
                    .is_some_and(|n| Some(n) == bundled.file_name())
                && (selected.parent() == bundled.parent()
                    || selected.parent() == Some(std::path::Path::new(crate::tun::RELOCATE_DIR))));
        if !acceptable {
            return Err(PrivilegeError::UnsafeGrantTarget {
                path: selected,
                detail: "selected helper is neither the bundled nor the relocated helper".into(),
            });
        }
        return Ok(Selection::InPlace {
            helper: selected,
            wrapper: crate::tun::bundled_run_path(),
        });
    }

    let src = validated_source(&bundled)?;
    Ok(Selection::Relocate {
        dest_dir: PathBuf::from(crate::tun::RELOCATE_DIR),
        dst: crate::tun::relocated_helper_path(),
        src,
    })
}

fn paths_equivalent(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Pure classifier for one mutation target's metadata. The detail strings are
/// user-visible refusal texts and must not drift from the elevated script.
fn file_target_violation(
    is_symlink: bool,
    is_regular: bool,
    uid: u32,
    mode: u32,
) -> Option<&'static str> {
    if is_symlink || !is_regular {
        return Some("target is not a non-symlink regular file");
    }
    if uid != 0 {
        return Some("target is not root-owned");
    }
    if mode & 0o022 != 0 {
        return Some("target is writable by non-root accounts");
    }
    None
}

fn unsafe_target(path: &Path, detail: &str) -> PrivilegeError {
    PrivilegeError::UnsafeGrantTarget {
        path: path.to_path_buf(),
        detail: detail.to_string(),
    }
}

/// Validates one existing mutation target and records its canonical path and
/// identity. Never chown a user-owned or writable file: a descriptor opened
/// before the change could survive it.
fn validate_mutation_target(path: &Path) -> Result<TargetId, PrivilegeError> {
    let lstat = std::fs::symlink_metadata(path)
        .map_err(|_| unsafe_target(path, "target is not a non-symlink regular file"))?;
    let canonical = std::fs::canonicalize(path)
        .map_err(|_| unsafe_target(path, "target is not a non-symlink regular file"))?;
    let meta = std::fs::metadata(&canonical)
        .map_err(|_| unsafe_target(path, "target is not a non-symlink regular file"))?;
    if let Some(detail) = file_target_violation(
        lstat.file_type().is_symlink(),
        meta.is_file(),
        meta.uid(),
        meta.mode(),
    ) {
        return Err(unsafe_target(path, detail));
    }
    require_protected_parents(&canonical)?;
    Ok(TargetId {
        path: canonical,
        identity: format!("{}:{}", meta.dev(), meta.ino()),
    })
}

/// Every canonical parent through `/` must be a root-owned nonsymlink
/// directory no non-root account can write through.
fn require_protected_parents(canonical: &Path) -> Result<(), PrivilegeError> {
    for parent in canonical.ancestors().skip(1) {
        if parent.as_os_str().is_empty() {
            break;
        }
        check_protected_dir(parent)?;
    }
    Ok(())
}

fn check_protected_dir(dir: &Path) -> Result<(), PrivilegeError> {
    let meta = std::fs::symlink_metadata(dir)
        .map_err(|e| unsafe_target(dir, &format!("cannot verify parent directory: {e}")))?;
    let is_dir = meta.is_dir() && !meta.file_type().is_symlink();
    if !is_dir || meta.uid() != 0 || meta.mode() & 0o022 != 0 {
        return Err(unsafe_target(dir, "parent directory is not protected"));
    }
    Ok(())
}

/// Validates the relocation destination: an existing directory must already be
/// protected, and a missing one may only be created below the deepest existing
/// protected ancestor. Attacker-owned or writable directories are never
/// adopted.
fn validate_destination(dest_dir: &Path) -> Result<(), PrivilegeError> {
    let mut probe = dest_dir;
    loop {
        match std::fs::symlink_metadata(probe) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                probe = probe
                    .parent()
                    .ok_or_else(|| unsafe_target(dest_dir, "parent directory is not protected"))?;
            }
            Err(e) => {
                return Err(unsafe_target(
                    probe,
                    &format!("cannot verify parent directory: {e}"),
                ));
            }
        }
    }
    check_protected_dir(probe)?;
    let canonical = std::fs::canonicalize(probe)
        .map_err(|e| unsafe_target(probe, &format!("cannot resolve: {e}")))?;
    require_protected_parents(&canonical)
}

/// The destination helper's identity before elevation, or `absent`. Rechecked
/// inside the script so nothing can be swapped across the polkit prompt.
fn prior_identity(dst: &Path) -> Result<String, PrivilegeError> {
    match std::fs::symlink_metadata(dst) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("absent".to_string()),
        Err(e) => Err(unsafe_target(dst, &format!("cannot verify target: {e}"))),
        Ok(meta) => {
            if let Some(detail) = file_target_violation(
                meta.file_type().is_symlink(),
                meta.is_file(),
                meta.uid(),
                meta.mode(),
            ) {
                return Err(unsafe_target(dst, detail));
            }
            Ok(format!("{}:{}", meta.dev(), meta.ino()))
        }
    }
}

/// Group-class bits (`r-x`) the helper and wrapper modes need.
const ACL_TARGET_GROUP_MASK: u8 = 0o5;

/// Bound on the `getfacl` access-ACL probe. Killed and reaped on expiry.
const ACL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
#[derive(Debug, PartialEq, Eq)]
enum AclNamedMask {
    NoNamed,
    Mask(u8),
}

/// Parses `getfacl -a -c -n -E` output. Named entries require numeric
/// qualifiers and exactly one mask record; anything malformed, duplicated, or
/// incomplete is rejected.
fn parse_access_acl(text: &str) -> Option<AclNamedMask> {
    let mut mask: Option<u8> = None;
    let mut named = false;
    let mut seen_user_base = false;
    let mut seen_group_base = false;
    let mut seen_other_base = false;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(':').collect();
        match fields.as_slice() {
            ["user", "", p] => {
                if seen_user_base {
                    return None;
                }
                seen_user_base = true;
                perms_bits(p)?;
            }
            ["group", "", p] => {
                if seen_group_base {
                    return None;
                }
                seen_group_base = true;
                perms_bits(p)?;
            }
            ["other", "", p] => {
                if seen_other_base {
                    return None;
                }
                seen_other_base = true;
                perms_bits(p)?;
            }
            ["mask", "", p] => {
                if mask.is_some() {
                    return None;
                }
                mask = Some(perms_bits(p)?);
            }
            ["user", q, p] | ["group", q, p] => {
                if q.is_empty() || !q.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                named = true;
                perms_bits(p)?;
            }
            _ => return None,
        }
    }
    if !(seen_user_base && seen_group_base && seen_other_base) {
        return None;
    }
    match (named, mask) {
        (false, _) => Some(AclNamedMask::NoNamed),
        (true, Some(m)) => Some(AclNamedMask::Mask(m)),
        (true, None) => None,
    }
}

fn perms_bits(p: &str) -> Option<u8> {
    let b = p.as_bytes();
    if b.len() != 3 {
        return None;
    }
    let bit = |c: u8, want: u8| -> Option<u8> {
        if c == want {
            Some(1)
        } else if c == b'-' {
            Some(0)
        } else {
            None
        }
    };
    Some((bit(b[0], b'r')? << 2) | (bit(b[1], b'w')? << 1) | bit(b[2], b'x')?)
}

fn mask_broadens_named_access(old_mask: u8) -> bool {
    ACL_TARGET_GROUP_MASK & !old_mask != 0
}

/// Refuses a mode change that would activate dormant named ACL permissions.
/// Missing `getfacl`, a failed probe, or invalid output refuses before any
/// mutation; existing entries are never stripped.
fn acl_allows_mode_change(path: &Path) -> Result<(), PrivilegeError> {
    let text =
        getfacl_access_text(path).ok_or_else(|| unsafe_target(path, "cannot verify access ACL"))?;
    match parse_access_acl(&text) {
        None => Err(unsafe_target(path, "cannot verify access ACL")),
        Some(AclNamedMask::NoNamed) => Ok(()),
        Some(AclNamedMask::Mask(old)) if mask_broadens_named_access(old) => Err(unsafe_target(
            path,
            "mode change would broaden named ACL access",
        )),
        Some(AclNamedMask::Mask(_)) => Ok(()),
    }
}

fn getfacl_access_text(path: &Path) -> Option<String> {
    let mut child = Command::new("getfacl")
        .env("LC_ALL", "C")
        .args(["-a", "-c", "-n", "-E", "--"])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + ACL_PROBE_TIMEOUT;
    let stdout = drain(child.stdout.take());
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    };
    let out = collect_raw(&stdout, deadline)?;
    if !status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

fn collect_raw(rx: &mpsc::Receiver<Vec<u8>>, deadline: Instant) -> Option<Vec<u8>> {
    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(buf) => Some(buf),
        Err(mpsc::RecvTimeoutError::Timeout) | Err(mpsc::RecvTimeoutError::Disconnected) => None,
    }
}

/// Parses the elevation's entire stdout as the single installed-helper
/// identity record. Extra output, missing fields, or non-decimal digits all
/// fail verification.
fn parse_helper_identity(stdout: &str) -> Option<(u64, u64)> {
    let line = stdout.strip_suffix('\n')?;
    if line.contains('\n') {
        return None;
    }
    let rest = line.strip_prefix("helper-identity=")?;
    let (dev, ino) = rest.split_once(':')?;
    if dev.is_empty() || ino.is_empty() {
        return None;
    }
    if !dev.bytes().chain(ino.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((dev.parse().ok()?, ino.parse().ok()?))
}

fn grant_verification(path: &Path, detail: impl Into<String>) -> PrivilegeError {
    PrivilegeError::GrantVerification {
        path: path.to_path_buf(),
        detail: detail.into(),
    }
}

/// Postflight check on one provisioned file: same identity, regular,
/// `root:v2ray-rs`, exact mode.
fn verify_grant_target(path: &Path, identity: &str, mode: u32) -> Result<(), PrivilegeError> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| grant_verification(path, format!("cannot stat: {e}")))?;
    if !meta.is_file() {
        return Err(grant_verification(path, "not a regular file"));
    }
    if meta.uid() != 0 {
        return Err(grant_verification(path, "not root-owned"));
    }
    if meta.mode() & 0o7777 != mode {
        return Err(grant_verification(
            path,
            format!("mode is {:o}, expected {:o}", meta.mode() & 0o7777, mode),
        ));
    }
    if format!("{}:{}", meta.dev(), meta.ino()) != identity {
        return Err(grant_verification(path, "target identity changed"));
    }
    let group_ok = nix::unistd::Group::from_name(HELPER_GROUP)
        .map_err(|e| grant_verification(path, format!("group lookup failed: {e}")))?
        .is_some_and(|g| u32::from(g.gid) == meta.gid());
    if !group_ok {
        return Err(grant_verification(
            path,
            format!("not owned by group {HELPER_GROUP}"),
        ));
    }
    Ok(())
}

fn verify_caps(path: &Path) -> Result<(), PrivilegeError> {
    match has_net_admin(path) {
        Ok(true) => Ok(()),
        Ok(false) => Err(grant_verification(path, "cap_net_admin is missing")),
        Err(e) => Err(grant_verification(path, e.to_string())),
    }
}

fn account_enrolled(user: &str) -> Result<bool, String> {
    match nix::unistd::Group::from_name(HELPER_GROUP) {
        Err(e) => Err(format!("group lookup failed: {e}")),
        Ok(None) => Ok(false),
        Ok(Some(group)) => Ok(group.mem.iter().any(|m| m == user)),
    }
}

fn verify_enrollment(user: &str, path: &Path) -> Result<(), PrivilegeError> {
    account_enrolled(user)
        .map_err(|detail| grant_verification(path, detail))
        .and_then(|enrolled| {
            enrolled.then_some(()).ok_or_else(|| {
                grant_verification(
                    path,
                    format!("account {user} is not enrolled in {HELPER_GROUP}"),
                )
            })
        })
}

fn verify_identity(target: &TargetId) -> Result<(), PrivilegeError> {
    let meta = std::fs::symlink_metadata(&target.path)
        .map_err(|e| grant_verification(&target.path, format!("cannot stat: {e}")))?;
    if !meta.is_file() {
        return Err(grant_verification(&target.path, "not a regular file"));
    }
    if format!("{}:{}", meta.dev(), meta.ino()) != target.identity {
        return Err(grant_verification(&target.path, "target identity changed"));
    }
    Ok(())
}

/// Canonicalizes the copy source and rejects anything that is not a regular
/// file sitting beside our own executable.
fn validated_source(src: &Path) -> Result<PathBuf, PrivilegeError> {
    let refuse = |why: &str| PrivilegeError::Probe(src.to_path_buf(), why.to_string());

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .and_then(|d| std::fs::canonicalize(d).ok())
        .ok_or_else(|| refuse("cannot resolve the executable's directory"))?;
    // Stat before canonicalizing: canonicalize resolves the final component too,
    // so a symlink test on its result could never be true.
    let meta = std::fs::symlink_metadata(src).map_err(|e| refuse(&format!("cannot stat: {e}")))?;
    let canonical =
        std::fs::canonicalize(src).map_err(|e| refuse(&format!("cannot resolve: {e}")))?;

    if !source_acceptable(
        &canonical,
        &exe_dir,
        meta.file_type().is_symlink(),
        meta.is_file(),
    ) {
        return Err(refuse("not a regular file beside the executable"));
    }
    Ok(canonical)
}

fn source_acceptable(
    canonical_src: &Path,
    exe_dir: &Path,
    is_symlink: bool,
    is_regular: bool,
) -> bool {
    !is_symlink && is_regular && canonical_src.starts_with(exe_dir)
}

/// The manual `setcap` command shown to the user when an automatic grant is not
/// possible.
pub fn manual_command(path: &Path, caps: &str) -> String {
    format!(
        "sudo setcap {} {}",
        shell_quote(caps),
        shell_quote(&path.display().to_string())
    )
}

/// Single-quotes `s` for a POSIX shell. The hint is meant to be pasted into a
/// root shell, so an embedded `'` must not end the quoting early.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Builds the `pkexec` argument vector. Every binary path is passed as a
/// positional parameter so the shell never parses it as code — a user-
/// controlled path containing shell metacharacters cannot inject commands.
///
/// Positionals above nine must be braced: `$10` would be read as `$1`
/// followed by a literal `0`.
fn grant_argv(plan: &GrantPlan) -> Vec<OsString> {
    let mut argv = vec![
        OsString::from("/bin/sh"),
        OsString::from("-c"),
        OsString::from(grant_script(plan)),
        OsString::from("sh"),
    ];

    match plan {
        GrantPlan::InPlace {
            backend,
            helper,
            wrapper,
            user,
            uid,
        } => {
            // $1 BACKEND_CAPS, $2 backend, $3 HELPER_CAPS, $4 helper,
            // $5 HELPER_GROUP, $6 account, $7 wrapper or empty,
            // $8 backend identity, $9 helper identity,
            // ${10} wrapper identity or empty, ${11} invoking UID.
            argv.push(OsString::from(BACKEND_CAPS));
            argv.push(backend.path.as_os_str().to_os_string());
            argv.push(OsString::from(HELPER_CAPS));
            argv.push(helper.path.as_os_str().to_os_string());
            argv.push(OsString::from(HELPER_GROUP));
            argv.push(OsString::from(user));
            argv.push(
                wrapper
                    .as_ref()
                    .map(|w| w.path.as_os_str().to_os_string())
                    .unwrap_or_default(),
            );
            argv.push(OsString::from(&backend.identity));
            argv.push(OsString::from(&helper.identity));
            argv.push(
                wrapper
                    .as_ref()
                    .map(|w| OsString::from(&w.identity))
                    .unwrap_or_default(),
            );
            argv.push(OsString::from(uid.to_string()));
        }
        GrantPlan::Relocate {
            backend,
            dest_dir,
            dst,
            user,
            uid,
            prior_dst,
            src: _,
        } => {
            // $1 BACKEND_CAPS, $2 backend, $3 dest dir, $4 HELPER_GROUP,
            // $5 destination, $6 HELPER_CAPS, $7 account,
            // $8 backend identity, $9 invoking UID,
            // ${10} prior destination identity or `absent`.
            argv.push(OsString::from(BACKEND_CAPS));
            argv.push(backend.path.as_os_str().to_os_string());
            argv.push(dest_dir.as_os_str().to_os_string());
            argv.push(OsString::from(HELPER_GROUP));
            argv.push(dst.as_os_str().to_os_string());
            argv.push(OsString::from(HELPER_CAPS));
            argv.push(OsString::from(user));
            argv.push(OsString::from(&backend.identity));
            argv.push(OsString::from(uid.to_string()));
            argv.push(OsString::from(prior_dst));
        }
    }
    argv
}

fn grant_script(plan: &GrantPlan) -> &'static str {
    match plan {
        // Every target is re-validated under elevation before the first
        // mutation: non-dereferencing stat, exact ownership and mode bits,
        // the recorded device:inode identity, and protected parents, so
        // nothing can be swapped across the polkit prompt. Ownership changes
        // precede modes and capability grants because chown clears both file
        // capabilities and the setuid bit.
        GrantPlan::InPlace { .. } => {
            "set -eu\n\
             refuse() { echo \"refusing TUN privilege grant: $1\" >&2; exit 1; }\n\
             check_file() {\n\
               [ ! -L \"$1\" ] && [ -f \"$1\" ] || refuse \"target is not a non-symlink regular file: $1\"\n\
               [ \"$(stat -c '%u' \"$1\")\" = 0 ] || refuse \"target is not root-owned: $1\"\n\
               [ \"$(( 0$(stat -c '%a' \"$1\") & 022 ))\" = 0 ] || refuse \"target is writable by non-root accounts: $1\"\n\
               [ \"$(stat -c '%d:%i' \"$1\")\" = \"$2\" ] || refuse \"target identity changed: $1\"\n\
             }\n\
             check_parents() {\n\
               p=$(dirname -- \"$1\")\n\
               while [ \"$p\" != / ] && [ -n \"$p\" ]; do\n\
                 [ ! -L \"$p\" ] && [ -d \"$p\" ] || refuse \"parent directory is not protected: $p\"\n\
                 [ \"$(stat -c '%u' \"$p\")\" = 0 ] || refuse \"parent directory is not protected: $p\"\n\
                 [ \"$(( 0$(stat -c '%a' \"$p\") & 022 ))\" = 0 ] || refuse \"parent directory is not protected: $p\"\n\
                 p=$(dirname -- \"$p\")\n\
               done\n\
             }\n\
             check_acl() {\n\
               acl=$(LC_ALL=C timeout 5 getfacl -a -c -n -E -- \"$1\" 2>/dev/null) || refuse \"cannot verify access ACL: $1\"\n\
               bu=0; bg=0; bo=0; bm=0; named=0\n\
               while IFS= read -r line; do\n\
                 case \"$line\" in\n\
                   '') ;;\n\
                   user::[r-][w-][x-]) bu=$((bu+1)) ;;\n\
                   group::[r-][w-][x-]) bg=$((bg+1)) ;;\n\
                   other::[r-][w-][x-]) bo=$((bo+1)) ;;\n\
                   mask::[r-][w-][x-]) bm=$((bm+1)) ;;\n\
                   user:*:*|group:*:*)\n\
                     rest=${line#*:}\n\
                     case \"$rest\" in *:*:*) refuse \"cannot verify access ACL: $1\" ;; esac\n\
                     q=${rest%%:*}\n\
                     case \"$q\" in ''|*[!0-9]*) refuse \"cannot verify access ACL: $1\" ;; esac\n\
                     p=${rest#*:}\n\
                     case \"$p\" in [r-][w-][x-]) ;; *) refuse \"cannot verify access ACL: $1\" ;; esac\n\
                     named=$((named+1)) ;;\n\
                   *) refuse \"cannot verify access ACL: $1\" ;;\n\
                 esac\n\
               done <<EOF
$acl
EOF\n\
               [ \"$bu\" = 1 ] && [ \"$bg\" = 1 ] && [ \"$bo\" = 1 ] || refuse \"cannot verify access ACL: $1\"\n\
               [ \"$bm\" -le 1 ] || refuse \"cannot verify access ACL: $1\"\n\
               [ \"$named\" != 0 ] || return 0\n\
               [ \"$bm\" = 1 ] || refuse \"cannot verify access ACL: $1\"\n\
               m=$(printf '%s\\n' \"$acl\" | sed -n 's/^mask::\\(...\\)$/\\1/p')\n\
               case \"$m\" in r?x) ;; *) refuse \"mode change would broaden named ACL access: $1\" ;; esac\n\
             }\n\
             [ \"$(id -u \"$6\")\" = \"${11}\" ] || refuse \"invoking account changed\"\n\
             [ \"${11}\" != 0 ] || refuse \"cannot enroll uid 0\"\n\
             check_file \"$2\" \"$8\"; check_parents \"$2\"\n\
             check_file \"$4\" \"$9\"; check_parents \"$4\"\n\
             if [ -n \"$7\" ]; then check_file \"$7\" \"${10}\"; check_parents \"$7\"; fi\n\
             check_acl \"$4\"\n\
             [ -z \"$7\" ] || check_acl \"$7\"\n\
             if getent group \"$5\" >/dev/null 2>&1; then :\n\
             else\n\
               rc=$?\n\
               [ \"$rc\" = 2 ] || refuse \"group lookup failed: $5\"\n\
               groupadd --system \"$5\" || refuse \"group creation failed: $5\"\n\
             fi\n\
             usermod -aG \"$5\" -- \"$6\" || refuse \"enrollment failed: $6\"\n\
             chown \"root:$5\" \"$4\" || refuse \"ownership change failed: $4\"\n\
             chmod 0750 \"$4\" || refuse \"mode change failed: $4\"\n\
             setcap \"$3\" \"$4\" || refuse \"capability grant failed: $4\"\n\
             if [ -n \"$7\" ]; then\n\
               chown \"root:$5\" \"$7\" || refuse \"ownership change failed: $7\"\n\
               chmod 4750 \"$7\" || refuse \"mode change failed: $7\"\n\
             fi\n\
             setcap \"$1\" \"$2\" || refuse \"capability grant failed: $2\"\n"
        }
        // The payload arrives on stdin, from a descriptor the unprivileged
        // caller opened: root cannot read an AppImage's FUSE mount at all.
        // `chown` precedes `chmod`/`setcap` because it clears both file
        // capabilities and the setuid bit. The redirect creates a fresh
        // inode, so no attribute of any previous file at that path survives.
        // The group is a dedicated system group rather than the caller's
        // primary group, which on distributions that share one (gid 100
        // `users`) would hand every local account a cap_net_admin binary.
        // The final stdout record reports the installed identity so the
        // unprivileged caller can verify it.
        GrantPlan::Relocate { .. } => {
            "set -eu\n\
             refuse() { echo \"refusing TUN privilege grant: $1\" >&2; exit 1; }\n\
             check_file() {\n\
               [ ! -L \"$1\" ] && [ -f \"$1\" ] || refuse \"target is not a non-symlink regular file: $1\"\n\
               [ \"$(stat -c '%u' \"$1\")\" = 0 ] || refuse \"target is not root-owned: $1\"\n\
               [ \"$(( 0$(stat -c '%a' \"$1\") & 022 ))\" = 0 ] || refuse \"target is writable by non-root accounts: $1\"\n\
               [ \"$(stat -c '%d:%i' \"$1\")\" = \"$2\" ] || refuse \"target identity changed: $1\"\n\
             }\n\
             check_parents() {\n\
               p=$(dirname -- \"$1\")\n\
               while [ \"$p\" != / ] && [ -n \"$p\" ]; do\n\
                 [ ! -L \"$p\" ] && [ -d \"$p\" ] || refuse \"parent directory is not protected: $p\"\n\
                 [ \"$(stat -c '%u' \"$p\")\" = 0 ] || refuse \"parent directory is not protected: $p\"\n\
                 [ \"$(( 0$(stat -c '%a' \"$p\") & 022 ))\" = 0 ] || refuse \"parent directory is not protected: $p\"\n\
                 p=$(dirname -- \"$p\")\n\
               done\n\
             }\n\
             check_dir() {\n\
               [ ! -L \"$1\" ] && [ -d \"$1\" ] || refuse \"parent directory is not protected: $1\"\n\
               [ \"$(stat -c '%u' \"$1\")\" = 0 ] || refuse \"parent directory is not protected: $1\"\n\
               [ \"$(( 0$(stat -c '%a' \"$1\") & 022 ))\" = 0 ] || refuse \"parent directory is not protected: $1\"\n\
             }\n\
             [ \"$(id -u \"$7\")\" = \"$9\" ] || refuse \"invoking account changed\"\n\
             [ \"$9\" != 0 ] || refuse \"cannot enroll uid 0\"\n\
             check_file \"$2\" \"$8\"; check_parents \"$2\"\n\
             if [ -e \"$3\" ]; then\n\
               check_dir \"$3\"; check_parents \"$3\"\n\
             else\n\
               p=\"$3\"\n\
               while [ ! -e \"$p\" ]; do p=$(dirname -- \"$p\"); done\n\
               check_dir \"$p\"; check_parents \"$p\"\n\
               mkdir -p \"$3\" || refuse \"destination creation failed: $3\"\n\
             fi\n\
             if [ -e \"$5\" ] || [ -L \"$5\" ]; then\n\
               [ \"${10}\" != absent ] || refuse \"target identity changed: $5\"\n\
               [ ! -L \"$5\" ] && [ -f \"$5\" ] || refuse \"target is not a non-symlink regular file: $5\"\n\
               [ \"$(stat -c '%d:%i' \"$5\")\" = \"${10}\" ] || refuse \"target identity changed: $5\"\n\
             else\n\
               [ \"${10}\" = absent ] || refuse \"target identity changed: $5\"\n\
             fi\n\
             if getent group \"$4\" >/dev/null 2>&1; then :\n\
             else\n\
               rc=$?\n\
               [ \"$rc\" = 2 ] || refuse \"group lookup failed: $4\"\n\
               groupadd --system \"$4\" || refuse \"group creation failed: $4\"\n\
             fi\n\
             usermod -aG \"$4\" -- \"$7\" || refuse \"enrollment failed: $7\"\n\
             rm -f \"$5\"\n\
             (umask 077; cat > \"$5\") || refuse \"copy failed: $5\"\n\
             chown \"root:$4\" \"$5\" || refuse \"ownership change failed: $5\"\n\
             chmod 0750 \"$5\" || refuse \"mode change failed: $5\"\n\
             setcap \"$6\" \"$5\" || refuse \"capability grant failed: $5\"\n\
             setcap \"$1\" \"$2\" || refuse \"capability grant failed: $2\"\n\
             printf 'helper-identity=%s:%s\\n' \"$(stat -c '%d' \"$5\")\" \"$(stat -c '%i' \"$5\")\"\n"
        }
    }
}

fn getcap_has_cap(getcap_output: &str, cap: &str) -> bool {
    let lower = getcap_output.trim().to_ascii_lowercase();
    // getcap prints `<path> <capset>`; match the cap as a whole token in the
    // trailing capset so a path component like `.../cap_net_admin/...` cannot
    // produce a false positive.
    let capset = lower
        .rsplit_once(char::is_whitespace)
        .map(|(_, capset)| capset)
        .unwrap_or(lower.as_str());
    capset
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|token| token == cap)
}

/// Ordered paths the grant elevation needs capability support on, each paired
/// with the caps a manual `setcap` would apply.
///
/// A relocating plan checks the destination directory, not the bundled copy: the
/// bundled copy's mount is known to be `nosuid`, which is the whole reason for
/// relocating.
fn preflight_targets<'a>(backend: &'a Path, plan: &'a Selection) -> Vec<(&'a Path, &'static str)> {
    let mut targets = vec![(backend, BACKEND_CAPS)];
    match plan {
        Selection::InPlace { helper, wrapper } => {
            targets.push((helper.as_path(), HELPER_CAPS));
            if let Some(wp) = wrapper {
                targets.push((wp.as_path(), HELPER_CAPS));
            }
        }
        Selection::Relocate { dest_dir, .. } => {
            targets.push((dest_dir.as_path(), HELPER_CAPS));
        }
    }
    targets
}

/// Best-effort check that the filesystem holding `path` honors file
/// capabilities. A `nosuid` mount silently ignores them. Defaults to `true`
/// when the mount cannot be determined.
pub(crate) fn file_caps_supported(path: &Path) -> bool {
    let Ok(mounts) = std::fs::read_to_string("/proc/self/mounts") else {
        return true;
    };
    !mount_for_path(&mounts, path).is_some_and(|opts| opts.split(',').any(|o| o == "nosuid"))
}

/// Returns the mount options of the longest mount-point prefix of `path`.
pub(crate) fn mount_for_path(mounts: &str, path: &Path) -> Option<String> {
    let path = path.to_string_lossy();
    let mut best: Option<(usize, String)> = None;
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (_dev, mount_point, _fs, opts) = (
            fields.next()?,
            fields.next()?,
            fields.next()?,
            fields.next()?,
        );
        let mount_point = unescape_mount(mount_point);
        let mount_point = mount_point.as_str();
        let covers = path == mount_point
            || (path.starts_with(mount_point)
                && (mount_point == "/" || path.as_bytes().get(mount_point.len()) == Some(&b'/')));
        // `>=` so that among duplicate entries for one mount point the last
        // wins, which is the one the kernel actually has mounted there.
        if covers
            && best
                .as_ref()
                .is_none_or(|(len, _)| mount_point.len() >= *len)
        {
            best = Some((mount_point.len(), opts.to_string()));
        }
    }
    best.map(|(_, opts)| opts)
}

/// `/proc/self/mounts` octal-escapes space, tab, newline and backslash. Without
/// undoing that, a path under `/mnt/My Drive` never matches its own mount line
/// and silently inherits the options of `/`.
fn unescape_mount(field: &str) -> String {
    if !field.contains('\\') {
        return field.to_string();
    }
    let bytes = field.as_bytes();
    let mut out = String::with_capacity(field.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = (bytes[i] == b'\\' && i + 3 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 4]).ok())
            .flatten()
            .and_then(|d| u8::from_str_radix(d, 8).ok());
        match octal {
            Some(byte) => {
                out.push(byte as char);
                i += 4;
            }
            None => {
                out.push(bytes[i] as char);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cap_in_getcap_output() {
        assert!(getcap_has_cap(
            "/usr/bin/xray cap_net_admin,cap_net_bind_service,cap_net_raw=ep",
            "cap_net_admin"
        ));
        assert!(getcap_has_cap(
            "/usr/bin/xray cap_net_admin+ep",
            "cap_net_admin"
        ));
        // getcap terminates its line with a newline; the trailing whitespace
        // must not swallow the capset.
        assert!(getcap_has_cap(
            "/usr/bin/xray cap_net_bind_service,cap_net_admin,cap_net_raw=ep\n",
            "cap_net_admin"
        ));
    }

    #[test]
    fn missing_cap_in_getcap_output() {
        assert!(!getcap_has_cap("", "cap_net_admin"));
        assert!(!getcap_has_cap(
            "/usr/bin/xray cap_net_bind_service=ep",
            "cap_net_admin"
        ));
    }

    #[test]
    fn getcap_does_not_match_cap_in_path() {
        // A path component must not be mistaken for a granted capability.
        assert!(!getcap_has_cap(
            "/var/cap_net_admin/xray cap_net_bind_service=ep",
            "cap_net_admin"
        ));
        // ...but the real capability still matches.
        assert!(getcap_has_cap(
            "/var/cap_net_admin/xray cap_net_admin+ep",
            "cap_net_admin"
        ));
    }

    fn target(path: &str, identity: &str) -> TargetId {
        TargetId {
            path: PathBuf::from(path),
            identity: identity.into(),
        }
    }

    fn in_place(wrapper: Option<&str>) -> GrantPlan {
        GrantPlan::InPlace {
            backend: target("/usr/bin/xray", "8:1"),
            helper: target("/usr/bin/v2ray-rs-netctl", "8:2"),
            wrapper: wrapper.map(|w| target(w, "8:3")),
            user: "zhuk".into(),
            uid: 1000,
        }
    }

    fn relocate() -> GrantPlan {
        GrantPlan::Relocate {
            backend: target("/usr/bin/xray", "8:1"),
            dest_dir: PathBuf::from("/usr/local/lib/v2ray-rs"),
            src: PathBuf::from("/tmp/.mount_abc/usr/bin/v2ray-rs-netctl"),
            dst: PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-netctl"),
            prior_dst: "absent".into(),
            user: "zhuk".into(),
            uid: 1000,
        }
    }

    fn sel_in_place(wrapper: Option<&str>) -> Selection {
        Selection::InPlace {
            helper: PathBuf::from("/usr/bin/v2ray-rs-netctl"),
            wrapper: wrapper.map(PathBuf::from),
        }
    }

    fn sel_relocate() -> Selection {
        Selection::Relocate {
            dest_dir: PathBuf::from("/usr/local/lib/v2ray-rs"),
            src: PathBuf::from("/tmp/.mount_abc/usr/bin/v2ray-rs-netctl"),
            dst: PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-netctl"),
        }
    }

    fn strs(argv: &[OsString]) -> Vec<&str> {
        argv.iter().map(|a| a.to_str().unwrap()).collect()
    }

    #[test]
    fn inplace_argv_addresses_double_digit_positionals() {
        let plan = in_place(None);
        let argv_raw = grant_argv(&plan);
        let argv = strs(&argv_raw);
        assert_eq!(argv.len(), 4 + 11);
        assert_eq!(
            argv[4..],
            [
                BACKEND_CAPS,
                "/usr/bin/xray",
                HELPER_CAPS,
                "/usr/bin/v2ray-rs-netctl",
                HELPER_GROUP,
                "zhuk",
                "",
                "8:1",
                "8:2",
                "",
                "1000",
            ]
        );
        let script = grant_script(&plan);
        // `$10` unbraced would read as `$1` and a literal `0`.
        assert!(script.contains("\"${10}\"") && script.contains("\"${11}\""));
    }

    #[test]
    fn inplace_argv_carries_wrapper_path_and_identity() {
        let plan = in_place(Some("/usr/bin/v2ray-rs-run"));
        let argv_raw = grant_argv(&plan);
        let argv = strs(&argv_raw);
        assert_eq!(argv.len(), 4 + 11);
        assert_eq!(argv[10], "/usr/bin/v2ray-rs-run");
        assert_eq!(argv[13], "8:3");
    }

    #[test]
    fn grant_argv_neutralizes_shell_metacharacters_in_path() {
        // A malicious backend path must be passed verbatim as a positional
        // argument, never interpolated into the executed script.
        let mut plan = in_place(Some("/tmp/run; chmod 777 /etc; #"));
        let GrantPlan::InPlace { backend, .. } = &mut plan else {
            unreachable!()
        };
        *backend = target("/tmp/x'; touch /pwned; '", "8:1");
        let argv_raw = grant_argv(&plan);
        assert_eq!(argv_raw[2].to_str().unwrap(), grant_script(&plan));
        assert_eq!(argv_raw[5].to_str().unwrap(), "/tmp/x'; touch /pwned; '");
        assert_eq!(
            argv_raw[10].to_str().unwrap(),
            "/tmp/run; chmod 777 /etc; #"
        );
    }

    #[test]
    fn relocating_grant_copies_then_grants_the_destination() {
        let plan = relocate();
        let argv_raw = grant_argv(&plan);
        let argv = strs(&argv_raw);
        assert_eq!(argv.len(), 4 + 10);
        assert_eq!(
            argv[4..],
            [
                BACKEND_CAPS,
                "/usr/bin/xray",
                "/usr/local/lib/v2ray-rs",
                HELPER_GROUP,
                "/usr/local/lib/v2ray-rs/v2ray-rs-netctl",
                HELPER_CAPS,
                "zhuk",
                "8:1",
                "1000",
                "absent",
            ]
        );
        // The source is the stdin payload now; it never reaches the shell.
        assert!(!argv.contains(&"/tmp/.mount_abc/usr/bin/v2ray-rs-netctl"));
    }

    #[test]
    fn relocating_grant_keeps_metachar_paths_positional() {
        let mut plan = relocate();
        let GrantPlan::Relocate { dst, .. } = &mut plan else {
            unreachable!()
        };
        *dst = PathBuf::from("/tmp/x\"; touch /pwned; \"");
        let argv_raw = grant_argv(&plan);
        assert_eq!(argv_raw[2].to_str().unwrap(), grant_script(&plan));
        assert_eq!(argv_raw[8].to_str().unwrap(), "/tmp/x\"; touch /pwned; \"");
    }

    #[test]
    fn relocating_script_never_reads_the_source_as_root() {
        let script = grant_script(&relocate());
        // The payload arrives on stdin because root cannot read the FUSE mount
        // the source lives on; any command naming it would fail with EACCES.
        assert!(script.contains("cat > \"$5\""));
        assert!(!script.contains("cp "));
    }

    #[test]
    fn relocating_script_targets_the_destination_only() {
        let script = grant_script(&relocate());
        assert!(script.contains("setcap \"$6\" \"$5\""));
        assert!(script.contains("chmod 0750 \"$5\""));
        // chown clears caps and the setuid bit, so it must come first.
        assert!(script.find("chown \"root:$4\"").unwrap() < script.find("chmod 0750").unwrap());
        // A fresh inode, so no attribute of a previous file at that path survives.
        assert!(script.find("rm -f \"$5\"").unwrap() < script.find("cat > \"$5\"").unwrap());
    }

    #[test]
    fn relocating_script_restricts_to_a_dedicated_group() {
        let script = grant_script(&relocate());
        // Never the caller's primary group: where distributions share one, that
        // would hand every local account a cap_net_admin binary.
        assert!(script.contains("groupadd --system \"$4\""));
        assert!(script.contains("chown \"root:$4\" \"$5\""));
        // Enrollment is unconditional: a session that has not picked up the
        // group yet must not skip the persistent account change.
        assert!(script.contains("usermod -aG \"$4\" -- \"$7\""));
        assert!(!script.contains("id -nG"));
    }

    #[test]
    fn relocating_script_reports_the_installed_identity_last() {
        let script = grant_script(&relocate());
        let last = script.trim_end().lines().last().unwrap();
        assert!(
            last.starts_with("printf 'helper-identity=%s:%s\\n'"),
            "{last}"
        );
    }

    #[test]
    fn inplace_script_validates_every_target_before_the_first_mutation() {
        let script = grant_script(&in_place(None));
        // Identities recorded pre-elevation are compared under root before
        // anything is modified.
        assert!(script.contains("stat -c '%d:%i'"));
        for gate in [
            "check_file \"$2\" \"$8\"",
            "check_file \"$4\" \"$9\"",
            "usermod -aG \"$5\" -- \"$6\"",
        ] {
            assert!(script.contains(gate), "missing {gate}");
        }
        let validate = script.find("usermod").unwrap();
        assert!(script.find("check_file \"$4\" \"$9\"").unwrap() < validate);
        // Ownership changes precede modes and capability grants.
        assert!(
            script.find("chown \"root:$5\" \"$4\"").unwrap()
                < script.find("chmod 0750 \"$4\"").unwrap()
        );
        assert!(
            script.find("chmod 0750 \"$4\"").unwrap()
                < script.find("setcap \"$3\" \"$4\"").unwrap()
        );
        // Only a missing group key (getent status 2) may create the group.
        assert!(script.contains("[ \"$rc\" = 2 ]"));
        // The script re-resolves the account and pins the invoking UID.
        assert!(script.contains("id -u \"$6\""));
        // The ACL probe is bounded and repeated under elevation.
        assert!(script.contains("timeout 5 getfacl"));
    }

    #[test]
    fn root_caller_is_refused_before_any_planning() {
        assert!(ensure_not_root(0, 0).is_err());
        assert!(ensure_not_root(0, 1000).is_err());
        assert!(ensure_not_root(1000, 0).is_err());
        assert!(ensure_not_root(1000, 1000).is_ok());
        assert!(matches!(
            ensure_not_root(0, 1000),
            Err(PrivilegeError::RootEnrollment)
        ));
    }

    #[test]
    fn unsafe_target_details_are_exact() {
        let not_a_file = "target is not a non-symlink regular file";
        assert_eq!(
            file_target_violation(true, true, 0, 0o750),
            Some(not_a_file)
        );
        assert_eq!(
            file_target_violation(false, false, 0, 0o750),
            Some(not_a_file)
        );
        assert_eq!(
            file_target_violation(false, true, 1000, 0o750),
            Some("target is not root-owned")
        );
        assert_eq!(
            file_target_violation(false, true, 0, 0o770),
            Some("target is writable by non-root accounts")
        );
        assert_eq!(
            file_target_violation(false, true, 0, 0o757),
            Some("target is writable by non-root accounts")
        );
        assert_eq!(file_target_violation(false, true, 0, 0o750), None);
        assert_eq!(file_target_violation(false, true, 0, 0o4750), None);
    }

    #[test]
    fn unsafe_target_refuses_real_files_owned_by_the_caller() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("helper");
        std::fs::write(&file, b"bin").unwrap();

        let err = validate_mutation_target(&file).unwrap_err();
        assert!(
            matches!(
                &err,
                PrivilegeError::UnsafeGrantTarget { detail, .. }
                    if detail == "target is not root-owned"
            ),
            "{err:?}"
        );

        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        let err = validate_mutation_target(&link).unwrap_err();
        assert!(
            matches!(
                &err,
                PrivilegeError::UnsafeGrantTarget { detail, .. }
                    if detail == "target is not a non-symlink regular file"
            ),
            "{err:?}"
        );
    }

    #[test]
    fn acl_parser_accepts_plain_and_named_records() {
        assert_eq!(
            parse_access_acl("user::rw-\ngroup::r--\nother::r--\n"),
            Some(AclNamedMask::NoNamed)
        );
        assert_eq!(
            parse_access_acl("user::rwx\nuser:1000:r-x\ngroup::r-x\nmask::r-x\nother::---\n"),
            Some(AclNamedMask::Mask(0o5))
        );
        assert_eq!(
            parse_access_acl("user::rwx\ngroup::r-x\nuser:0:---\nmask::rwx\nother::---\n"),
            Some(AclNamedMask::Mask(0o7))
        );
        // A named entry without a mask record is incomplete.
        assert_eq!(
            parse_access_acl("user::rwx\nuser:1000:r-x\nother::---\n"),
            None
        );
        // Real multi-line base-only ACL with a trailing blank line.
        assert_eq!(
            parse_access_acl("user::rw-\ngroup::r--\nother::---\n\n"),
            Some(AclNamedMask::NoNamed)
        );
        // A named group entry with a real base-only prefix.
        assert_eq!(
            parse_access_acl("user::rwx\ngroup::r-x\ngroup:1000:r-x\nmask::r-x\nother::---\n"),
            Some(AclNamedMask::Mask(0o5))
        );
        // Two-part base records are not a real getfacl shape.
        assert_eq!(parse_access_acl("user:rw-\nother::r--\n"), None);
        // Missing a base record.
        assert_eq!(parse_access_acl("user::rwx\ngroup::r-x\n"), None);
        // Duplicate base record.
        assert_eq!(
            parse_access_acl("user::rwx\nuser::r-x\ngroup::r-x\nother::---\n"),
            None
        );
        // Two-field mask shape is not a real getfacl record.
        assert_eq!(parse_access_acl("user::rwx\nmask:r-x\nother::---\n"), None);
    }

    #[test]
    fn acl_parser_rejects_malformed_records() {
        // Duplicate mask.
        assert_eq!(parse_access_acl("mask::r-x\nmask::r-x\n"), None);
        // Permissions must be exactly three position-valid characters.
        assert_eq!(parse_access_acl("user::rw\n"), None);
        assert_eq!(parse_access_acl("user::xrw\n"), None);
        // Named qualifiers must be numeric.
        assert_eq!(parse_access_acl("user:zhuk:r-x\nmask::r-x\n"), None);
        // Unknown record shape.
        assert_eq!(parse_access_acl("default:user::rwx\n"), None);
        assert_eq!(parse_access_acl("junk\n"), None);
        assert_eq!(parse_access_acl("user:1:2:3\n"), None);
    }

    #[test]
    fn acl_mask_decision_blocks_dormant_named_access() {
        // The helper/wrapper modes need group r-x; an old mask that lacks
        // either bit would be broadened by the mode change.
        assert!(mask_broadens_named_access(0));
        assert!(mask_broadens_named_access(0o4));
        assert!(mask_broadens_named_access(0o1));
        assert!(!mask_broadens_named_access(0o5));
        assert!(!mask_broadens_named_access(0o7));
    }

    #[test]
    fn helper_identity_record_must_be_the_whole_stdout() {
        assert_eq!(
            parse_helper_identity("helper-identity=8:42\n"),
            Some((8, 42))
        );
        // Leading zeros are still plain decimal digits.
        assert_eq!(
            parse_helper_identity("helper-identity=08:042\n"),
            Some((8, 42))
        );
        assert_eq!(parse_helper_identity("helper-identity=8:42"), None);
        assert_eq!(parse_helper_identity("helper-identity=8:42\nextra\n"), None);
        assert_eq!(
            parse_helper_identity("polkit: granted\nhelper-identity=8:42\n"),
            None
        );
        assert_eq!(parse_helper_identity("helper-identity=8:42:9\n"), None);
        assert_eq!(parse_helper_identity("helper-identity=8:0x1f\n"), None);
        assert_eq!(parse_helper_identity("helper-identity=:42\n"), None);
        assert_eq!(parse_helper_identity(""), None);
    }

    #[test]
    fn source_must_be_a_regular_file_beside_the_executable() {
        let dir = Path::new("/tmp/.mount_abc/usr/bin");
        let good = dir.join("v2ray-rs-netctl");

        assert!(source_acceptable(&good, dir, false, true));
        // A symlink beside the executable is refused outright: the caller stats
        // the path as given, before canonicalization resolves it away.
        assert!(!source_acceptable(&good, dir, true, true));
        assert!(!source_acceptable(&good, dir, false, false));
        // Anything outside the executable's own directory is refused.
        assert!(!source_acceptable(
            Path::new("/etc/shadow"),
            dir,
            false,
            true
        ));
    }

    #[test]
    fn manual_hint_quotes_every_path_shape() {
        let cases = [
            ("/usr/bin/xray", "'/usr/bin/xray'"),
            ("/opt/my apps/xray", "'/opt/my apps/xray'"),
            ("/opt/x';id;'/xray", r"'/opt/x'\'';id;'\''/xray'"),
            ("/opt/$(id)/xray", "'/opt/$(id)/xray'"),
        ];
        for (path, quoted) in cases {
            let path = Path::new(path);
            let command = manual_command(path, BACKEND_CAPS);
            assert_eq!(command, format!("sudo setcap '{BACKEND_CAPS}' {quoted}"));
            let text = PrivilegeError::Unsupported {
                path: path.to_path_buf(),
                caps: BACKEND_CAPS.to_string(),
            }
            .to_string();
            assert!(text.ends_with(&command), "{text}");
        }
    }

    #[test]
    fn manual_command_format() {
        assert_eq!(
            manual_command(Path::new("/usr/bin/xray"), BACKEND_CAPS),
            "sudo setcap 'cap_net_admin,cap_net_bind_service,cap_net_raw+ep' '/usr/bin/xray'"
        );
    }

    #[test]
    fn mount_lookup_picks_longest_prefix() {
        let mounts = "\
/dev/sda1 / ext4 rw,relatime 0 0
tmpfs /tmp tmpfs rw,nosuid,nodev 0 0
/dev/sda2 /home ext4 rw,relatime 0 0
";
        assert_eq!(
            mount_for_path(mounts, Path::new("/usr/bin/xray")).as_deref(),
            Some("rw,relatime")
        );
        assert!(
            mount_for_path(mounts, Path::new("/tmp/x"))
                .unwrap()
                .contains("nosuid")
        );
        // `/tmpfoo` must not match the `/tmp` mount.
        assert_eq!(
            mount_for_path(mounts, Path::new("/tmpfoo/x")).as_deref(),
            Some("rw,relatime")
        );
    }

    #[test]
    fn nosuid_mount_is_unsupported() {
        let mounts = "tmpfs /mnt/ro tmpfs rw,nosuid 0 0\n/dev/sda1 / ext4 rw 0 0\n";
        assert!(
            mount_for_path(mounts, Path::new("/mnt/ro/xray"))
                .unwrap()
                .split(',')
                .any(|o| o == "nosuid")
        );
    }

    #[test]
    fn nosuid_mount_drives_the_unsupported_error_text() {
        let mounts = "\
/dev/sda1 / ext4 rw 0 0
tmpfs /tmp/.mount_abc tmpfs rw,nosuid,nodev 0 0
";
        let backend = Path::new("/tmp/.mount_abc/usr/bin/xray");
        let opts = mount_for_path(mounts, backend).expect("backend sits under a mount");
        assert!(opts.split(',').any(|o| o == "nosuid"), "{opts}");

        // The manager carries this Display verbatim in its mount-unsupported
        // ProcessError, so the manual setcap wording must survive the trip.
        let text = PrivilegeError::Unsupported {
            path: backend.to_path_buf(),
            caps: BACKEND_CAPS.to_string(),
        }
        .to_string();
        assert!(text.contains("ignores file capabilities"), "{text}");
        assert!(
            text.ends_with(&manual_command(backend, BACKEND_CAPS)),
            "{text}"
        );
    }

    #[test]
    fn preflight_targets_includes_helper_and_optional_wrapper() {
        let backend = Path::new("/usr/bin/xray");
        let helper = Path::new("/usr/bin/v2ray-rs-netctl");
        let wrapper = Path::new("/usr/lib/v2ray-rs/v2ray-rs-run");

        let plan = sel_in_place(None);
        let without_wrapper = preflight_targets(backend, &plan);
        assert_eq!(without_wrapper.len(), 2);
        assert_eq!(without_wrapper[0].0, backend);
        assert_eq!(without_wrapper[0].1, BACKEND_CAPS);
        assert_eq!(without_wrapper[1].0, helper);
        assert_eq!(without_wrapper[1].1, HELPER_CAPS);

        let plan = sel_in_place(Some("/usr/lib/v2ray-rs/v2ray-rs-run"));
        let with_wrapper = preflight_targets(backend, &plan);
        assert_eq!(with_wrapper.len(), 3);
        assert_eq!(with_wrapper[2].0, wrapper);
        assert_eq!(with_wrapper[2].1, HELPER_CAPS);
    }

    #[test]
    fn relocating_preflight_checks_the_destination_not_the_appdir() {
        let plan = sel_relocate();
        let targets = preflight_targets(Path::new("/usr/bin/xray"), &plan);

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[1].0, Path::new("/usr/local/lib/v2ray-rs"));
        // Preflighting the bundled copy would always abort: its mount is nosuid,
        // which is precisely why the plan relocates.
        assert!(
            !targets
                .iter()
                .any(|(p, _)| p.starts_with("/tmp/.mount_abc"))
        );
    }

    #[test]
    fn mount_lookup_unescapes_octal_in_mount_points() {
        let mounts = "\
/dev/sda1 / ext4 rw,relatime 0 0
/dev/sdb1 /run/media/zhuk/My\\040Drive vfat rw,nosuid,nodev 0 0
";
        // Without un-escaping this falls back to / and reports caps supported.
        assert!(
            mount_for_path(mounts, Path::new("/run/media/zhuk/My Drive/xray"))
                .unwrap()
                .split(',')
                .any(|o| o == "nosuid")
        );
    }

    #[test]
    fn mount_lookup_prefers_the_last_of_duplicate_entries() {
        // Over-mounts are listed in mount order; the effective one is the last.
        let mounts = "\
rootfs / rootfs rw,nosuid 0 0
/dev/sda1 / ext4 rw,relatime 0 0
";
        assert_eq!(
            mount_for_path(mounts, Path::new("/usr/bin/xray")).as_deref(),
            Some("rw,relatime")
        );
    }

    #[test]
    fn mount_lookup_handles_a_destination_that_does_not_exist_yet() {
        let mounts = "/dev/sda1 / ext4 rw,relatime 0 0\n";
        assert_eq!(
            mount_for_path(mounts, Path::new("/usr/local/lib/v2ray-rs/v2ray-rs-netctl")).as_deref(),
            Some("rw,relatime")
        );
    }

    #[test]
    fn probe_error_text_pinpoints_timeout() {
        assert_eq!(
            probe_error_text(&ProbeFailure::Timeout),
            "could not verify TUN capabilities: getcap timed out"
        );
    }

    #[test]
    fn probe_error_text_names_libcap_when_getcap_is_missing() {
        assert_eq!(
            probe_error_text(&ProbeFailure::NotFound),
            "getcap not found; install libcap to use TUN"
        );
    }

    #[test]
    fn probe_error_text_interpolates_trimmed_stderr_on_non_zero_exit() {
        assert_eq!(
            probe_error_text(&ProbeFailure::Exit("exit status 1".into())),
            "could not verify TUN capabilities: getcap exited with exit status 1"
        );
        // An empty stderr still produces the pinned prefix verbatim.
        assert_eq!(
            probe_error_text(&ProbeFailure::Exit(String::new())),
            "could not verify TUN capabilities: getcap exited with "
        );
    }

    #[test]
    fn probe_error_text_says_getcap_could_not_run() {
        let err = std::io::Error::from_raw_os_error(nix::libc::EACCES);
        assert_eq!(
            probe_error_text(&ProbeFailure::Spawn(err.to_string())),
            format!("could not verify TUN capabilities: cannot run getcap: {err}")
        );
    }

    #[test]
    fn unrunnable_getcap_is_a_spawn_failure() {
        let dir = tempfile::TempDir::new().unwrap();
        let probe = probe_net_admin_within(dir.path(), Path::new("/usr/bin/xray"), GETCAP_TIMEOUT);
        assert!(
            matches!(
                probe,
                Err(PrivilegeError::ProbeFailure(ProbeFailure::Spawn(_)))
            ),
            "{probe:?}"
        );
    }

    fn sh_probe(dir: &Path, script: &str, timeout: Duration) -> Result<bool, PrivilegeError> {
        // `sh` reads the script as the probed path, so no freshly written file
        // is ever exec'd (and no ETXTBSY race with parallel tests).
        let path = dir.join("getcap.sh");
        std::fs::write(&path, script).unwrap();
        probe_net_admin_within(Path::new("/bin/sh"), &path, timeout)
    }

    #[test]
    fn probe_reads_the_capset_from_stdout() {
        let dir = tempfile::TempDir::new().unwrap();
        let probe = sh_probe(dir.path(), "echo \"$0 cap_net_admin+ep\"\n", GETCAP_TIMEOUT);
        assert!(probe.unwrap());
    }

    #[test]
    fn probe_output_drain_is_bounded_by_the_deadline() {
        let dir = tempfile::TempDir::new().unwrap();
        let pid_file = dir.path().join("pid");
        let script = format!("sleep 30 &\necho $! > {}\nexit 0\n", pid_file.display());

        let started = Instant::now();
        let probe = sh_probe(dir.path(), &script, Duration::from_millis(300));
        let elapsed = started.elapsed();

        if let Ok(pid) = std::fs::read_to_string(&pid_file) {
            let pid = nix::unistd::Pid::from_raw(pid.trim().parse().unwrap());
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
        }
        assert!(
            matches!(
                probe,
                Err(PrivilegeError::ProbeFailure(ProbeFailure::Timeout))
            ),
            "{probe:?}"
        );
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
    }

    #[test]
    fn caps_check_needed_is_false_for_root() {
        assert!(!caps_check_needed(0));
    }

    #[test]
    fn caps_check_needed_is_true_for_a_normal_user() {
        assert!(caps_check_needed(1000));
    }
}
