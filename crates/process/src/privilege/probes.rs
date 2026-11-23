use super::grant::*;
use super::verify::*;
use super::*;
/// The capability names the relocated helper must hold exactly.
pub(super) const HELPER_REQUIRED_CAPS: [&str; 1] = ["cap_net_admin"];

/// The smallest capability question startup asks: can this binary exercise
/// TUN at all? The grant's post-flight verification answers a different,
/// larger question with the exact set each target must hold.
pub(super) const STARTUP_MINIMUM_CAPS: [&str; 1] = ["cap_net_admin"];

/// Where libcap2-bin installs `getcap` on Debian. A minimal desktop session
/// can keep the sbin directory off a normal user's PATH, and global PATH
/// changes are off the table.
pub(super) const GETCAP_FALLBACK: &str = "/usr/bin/getcap";

/// Resolves the probe's `getcap` without touching PATH: an absolute program
/// is used as given, a bare name is looked up on PATH and then at the
/// Debian-standard absolute location. `None` means no candidate exists at
/// all; a found-but-unrunnable binary surfaces later as a spawn failure, so
/// "absent" and "unreadable" stay distinguishable.
pub(super) fn resolve_getcap_program(program: &Path) -> Option<PathBuf> {
    if program.is_absolute() {
        return Some(program.to_path_buf());
    }
    let name = program.to_str()?;
    crate::tun::find_on_path(name).or_else(|| {
        let fallback = Path::new(GETCAP_FALLBACK);
        fallback.is_file().then(|| fallback.to_path_buf())
    })
}

/// Reports whether the binary at `path` already holds the startup minimum
/// (`cap_net_admin`, effective and permitted) in its file capabilities.
/// Bounded by `GETCAP_TIMEOUT`; a wedged probe is killed (not abandoned) so
/// it cannot outlive the start attempt.
pub fn has_net_admin(path: &Path) -> Result<bool, PrivilegeError> {
    probe_net_admin(Path::new("getcap"), path)
}

pub(crate) fn probe_net_admin(getcap: &Path, path: &Path) -> Result<bool, PrivilegeError> {
    probe_caps_within(getcap, path, &STARTUP_MINIMUM_CAPS, GETCAP_TIMEOUT)
}

#[cfg(test)]
pub(super) fn probe_net_admin_within(
    getcap: &Path,
    path: &Path,
    timeout: Duration,
) -> Result<bool, PrivilegeError> {
    probe_caps_within(getcap, path, &STARTUP_MINIMUM_CAPS, timeout)
}

#[cfg(test)]
pub(super) fn probe_antidpi_caps(path: &Path) -> Result<bool, PrivilegeError> {
    probe_caps_within(
        Path::new("getcap"),
        path,
        &ANTIDPI_REQUIRED_CAPS,
        GETCAP_TIMEOUT,
    )
}

/// One bounded `getcap` probe for any required capability set. The caller
/// passes an absolute probe program or a bare name to resolve.
pub(super) fn probe_caps_within(
    getcap: &Path,
    path: &Path,
    required: &[&str],
    timeout: Duration,
) -> Result<bool, PrivilegeError> {
    let resolved = resolve_getcap_program(getcap)
        .ok_or(PrivilegeError::ProbeFailure(ProbeFailure::NotFound))?;
    let mut child = Command::new(&resolved)
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

    Ok(caps_present(
        &String::from_utf8_lossy(&stdout),
        path,
        required,
    ))
}

/// Reports whether the anti-DPI binary at `path` holds every capability it
/// needs — both `cap_net_admin` and `cap_net_raw` with the effective and
/// permitted flags (`ep`). Bounded by `GETCAP_TIMEOUT`; effective UID 0
/// callers skip the probe (`caps_check_needed`), since root already holds
/// every capability.
pub fn has_antidpi_caps(path: &Path) -> Result<bool, PrivilegeError> {
    probe_caps_within(
        Path::new("getcap"),
        path,
        &ANTIDPI_REQUIRED_CAPS,
        GETCAP_TIMEOUT,
    )
}

/// Reads a pipe to EOF on its own thread. An exited `getcap` does not close
/// the pipe while a descendant still holds the inherited descriptor, so the
/// read must never run on the caller's deadline-bound path. A reader left
/// blocked that way ends once the last holder closes the descriptor.
pub(super) fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<Vec<u8>> {
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

pub(super) fn collect(
    output: &mpsc::Receiver<Vec<u8>>,
    deadline: Instant,
) -> Result<Vec<u8>, PrivilegeError> {
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
/// option when the bundled copy sits on a `nosuid` mount. The anti-DPI helper
/// rides along as an optional target in either plan.
///
/// An opened relocation source. The handle is opened without following a
/// final symlink, its identity and byte length are pinned from the open
/// file, and the elevated script receives exactly the pinned length — so
/// nothing can swap or grow the file between validation and the payload the
/// elevation consumes.
// Access ACL probing for grant targets.
/// Group-class bits (`r-x`) the helper and wrapper modes need.
pub(super) const ACL_TARGET_GROUP_MASK: u8 = 0o5;

/// Bound on the `getfacl` access-ACL probe. Killed and reaped on expiry.
pub(super) const ACL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
#[derive(Debug, PartialEq, Eq)]
pub(super) enum AclNamedMask {
    NoNamed,
    Mask(u8),
}

/// Parses `getfacl -a -c -n -E` output. Named entries require numeric
/// qualifiers and exactly one mask record; anything malformed, duplicated, or
/// incomplete is rejected.
pub(super) fn parse_access_acl(text: &str) -> Option<AclNamedMask> {
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

pub(super) fn perms_bits(p: &str) -> Option<u8> {
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

pub(super) fn mask_broadens_named_access(old_mask: u8) -> bool {
    ACL_TARGET_GROUP_MASK & !old_mask != 0
}

/// Refuses a mode change that would activate dormant named ACL permissions.
/// Missing `getfacl`, a failed probe, or invalid output refuses before any
/// mutation; existing entries are never stripped.
pub(super) fn acl_allows_mode_change(path: &Path) -> Result<(), PrivilegeError> {
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

pub(super) fn getfacl_access_text(path: &Path) -> Option<String> {
    let mut child = Command::new("getfacl")
        .env("LC_ALL", "C")
        .args(["-a", "-c", "-n", "-E", "--"])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
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

pub(super) fn collect_raw(rx: &mpsc::Receiver<Vec<u8>>, deadline: Instant) -> Option<Vec<u8>> {
    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(buf) => Some(buf),
        Err(mpsc::RecvTimeoutError::Timeout) | Err(mpsc::RecvTimeoutError::Disconnected) => None,
    }
}

/// Parses the elevation's entire stdout as the installed identity records:
pub(super) fn mount_opts_readonly(path: &Path) -> bool {
    let Ok(mounts) = std::fs::read_to_string("/proc/self/mounts") else {
        return false;
    };
    mount_for_path(&mounts, path).is_some_and(|opts| readonly_option(&opts))
}

pub(super) fn readonly_option(opts: &str) -> bool {
    opts.split(',').any(|option| option == "ro")
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

/// `file_caps_supported` for the mount holding `path`'s target. `path` is
/// canonicalized first: the kernel reads a file's capabilities from the
/// target inode, so a symlink's own directory is not the mount that
/// matters. An unresolvable path is checked as given.
pub(crate) fn target_caps_supported(path: &Path) -> bool {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    file_caps_supported(target.parent().unwrap_or(Path::new("/")))
}

/// Returns the mount options of the longest mount-point prefix of `path`.
pub(crate) fn mount_for_path(mounts: &str, path: &Path) -> Option<String> {
    let path = path.to_string_lossy();
    let mut best: Option<(usize, String)> = None;
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (Some(_), Some(mount_point), Some(_), Some(opts)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
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
pub(super) fn unescape_mount(field: &str) -> String {
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
