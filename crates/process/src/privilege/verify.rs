use super::grant::*;
use super::probes::*;
use super::*;
/// the route helper's line is required, the anti-DPI binary's line is present
/// exactly when the plan relocated it. Extra output, missing fields, or
/// non-decimal digits all fail verification.
pub(super) type GrantIdentity = (u64, u64);

pub(super) fn parse_grant_identities(
    stdout: &str,
) -> Option<(GrantIdentity, Option<GrantIdentity>)> {
    let body = stdout.strip_suffix('\n')?;
    let mut lines = body.split('\n');
    let helper = parse_identity_line(lines.next()?, "helper-identity=")?;
    let antidpi = match lines.next() {
        None => None,
        Some(line) => Some(parse_identity_line(line, "antidpi-identity=")?),
    };
    if lines.next().is_some() {
        return None;
    }
    Some((helper, antidpi))
}

pub(super) fn parse_identity_line(line: &str, prefix: &str) -> Option<(u64, u64)> {
    let rest = line.strip_prefix(prefix)?;
    let (dev, ino) = rest.split_once(':')?;
    if dev.is_empty() || ino.is_empty() {
        return None;
    }
    if !dev.bytes().chain(ino.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((dev.parse().ok()?, ino.parse().ok()?))
}

pub(super) fn grant_verification(path: &Path, detail: impl Into<String>) -> PrivilegeError {
    PrivilegeError::GrantVerification {
        path: path.to_path_buf(),
        detail: detail.into(),
    }
}

/// Postflight check on one provisioned file: same identity, regular,
/// `root:v2ray-rs`, exact mode.
pub(super) fn verify_grant_target(
    path: &Path,
    identity: &str,
    mode: u32,
) -> Result<(), PrivilegeError> {
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

pub(super) fn verify_caps(path: &Path) -> Result<(), PrivilegeError> {
    verify_caps_exact(path, &required_caps(HELPER_CAPS))
}

/// Post-grant verification checks the exact set the grant applied — all of
/// `BACKEND_CAPS`, not one name among them.
pub(super) fn verify_backend_caps(path: &Path) -> Result<(), PrivilegeError> {
    verify_caps_exact(path, &required_caps(BACKEND_CAPS))
}

pub(super) fn verify_antidpi_caps(path: &Path) -> Result<(), PrivilegeError> {
    verify_caps_exact(path, &ANTIDPI_REQUIRED_CAPS)
}

pub(super) fn verify_caps_exact(path: &Path, required: &[&str]) -> Result<(), PrivilegeError> {
    if required.is_empty() {
        return Err(grant_verification(path, "required capability set is empty"));
    }
    match probe_caps_within(Path::new("getcap"), path, required, GETCAP_TIMEOUT) {
        Ok(true) => Ok(()),
        Ok(false) => Err(grant_verification(
            path,
            format!("missing required capabilities ({})", required.join(", ")),
        )),
        Err(e) => Err(grant_verification(path, e.to_string())),
    }
}

/// The capability names a `setcap` clause like the constants above requires.
/// Flags must grant both effective and permitted; anything else yields an
/// empty set, which post-grant verification refuses rather than passes.
pub(super) fn required_caps(caps: &'static str) -> Vec<&'static str> {
    let Some((names, flags)) = caps.split_once('+') else {
        return Vec::new();
    };
    let known = flags.bytes().all(|flag| matches!(flag, b'e' | b'i' | b'p'));
    if !known || !flags.contains('e') || !flags.contains('p') {
        return Vec::new();
    }
    names.split(',').filter(|name| !name.is_empty()).collect()
}

pub(super) fn account_enrolled(user: &str) -> Result<bool, String> {
    match nix::unistd::Group::from_name(HELPER_GROUP) {
        Err(e) => Err(format!("group lookup failed: {e}")),
        Ok(None) => Ok(false),
        Ok(Some(group)) => Ok(group.mem.iter().any(|m| m == user)),
    }
}

pub(super) fn verify_enrollment(user: &str, path: &Path) -> Result<(), PrivilegeError> {
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

pub(super) fn verify_identity(target: &TargetId) -> Result<(), PrivilegeError> {
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

/// Canonicalizes the copy source, opens it without following a final
/// symlink, pins identity and length from the open file, and requires a
/// Strict, path-aware parser for capability requirements in `getcap` output.
/// Only the clause list following exactly `path` — the file `getcap` was
/// asked about — counts; capability names that appear only inside the path
/// are never a grant. A clause is `name[,name...] = flags` (or the
/// historical `names+flags`) with flags drawn from `eip`; a required
/// capability counts only when its clause carries both the effective (`e`)
/// and the permitted (`p`) flag. An empty name list (`=ep`) is libcap's
/// whole-set clause and carries every capability. Anything malformed — a
/// missing separator, an unknown flag, an empty or duplicated name, or a
/// line for a different path — leaves the file unprovisioned.
pub(super) fn caps_present(output: &str, path: &Path, required: &[&str]) -> bool {
    let line = output.trim_end_matches(['\r', '\n']);
    let rest = match line.strip_prefix(&*path.to_string_lossy()) {
        Some(rest) if rest.is_empty() || rest.starts_with([' ', '\t']) => rest,
        _ => return false,
    };
    let capset = rest.trim_start();
    if capset.is_empty() {
        // `getcap` prints nothing for a file without capabilities.
        return false;
    }
    let mut granted = vec![false; required.len()];
    if granted.is_empty() {
        return false;
    }
    let mut seen: Vec<&str> = Vec::new();
    for clause in capset.split_whitespace() {
        let Some((names, flags)) = clause.split_once(['=', '+']) else {
            return false;
        };
        if !flags.bytes().all(|flag| matches!(flag, b'e' | b'i' | b'p')) {
            return false;
        }
        let effective = flags.contains('e') && flags.contains('p');
        if names.is_empty() {
            granted.iter_mut().for_each(|slot| *slot = effective);
            continue;
        }
        for name in names.split(',') {
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return false;
            }
            if seen.contains(&name) {
                // libcap never repeats a capability across clauses.
                return false;
            }
            seen.push(name);
            if let Some(slot) = required.iter().position(|need| *need == name) {
                granted[slot] = effective;
            }
        }
    }
    granted[..required.len()].iter().all(|present| *present)
}

/// The anti-DPI pair through the shared parser.
#[cfg(test)]
pub(super) fn antidpi_caps_present(output: &str, path: &Path) -> bool {
    caps_present(output, path, &ANTIDPI_REQUIRED_CAPS)
}
