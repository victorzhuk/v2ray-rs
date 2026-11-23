use super::probes::*;
use super::verify::*;
use super::*;
pub(super) struct PinnedSource {
    pub(super) file: std::fs::File,
    pub(super) path: PathBuf,
    pub(super) identity: String,
    pub(super) len: u64,
}

impl fmt::Debug for PinnedSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinnedSource")
            .field("path", &self.path)
            .field("identity", &self.identity)
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(super) enum Selection {
    InPlace {
        helper: PathBuf,
        wrapper: Option<PathBuf>,
        antidpi: Option<PathBuf>,
    },
    Relocate {
        dest_dir: PathBuf,
        src: PinnedSource,
        dst: PathBuf,
        antidpi: Option<AntidpiRelocate>,
    },
}

/// The anti-DPI payload of a relocating grant: its bundled source is piped to
/// the elevated script beside the route helper's bytes, and the destination is
/// provisioned with the anti-DPI capability pair.
#[derive(Debug)]
pub(super) struct AntidpiRelocate {
    pub(super) src: PinnedSource,
    pub(super) dst: PathBuf,
    /// The destination's recorded `device:inode`, or `absent`; re-checked in
    /// the elevated script like the route helper's destination.
    pub(super) prior_dst: String,
}

/// A validated mutation target: its canonical path and decimal `device:inode`
/// identity, re-checked inside the elevated script so nothing can be swapped
/// across the polkit prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TargetId {
    pub(super) path: PathBuf,
    pub(super) identity: String,
}

/// A fully validated elevation: every path is canonical and carries the
/// identity the elevated script must re-observe before mutating anything.
#[derive(Debug)]
pub(super) enum GrantPlan {
    InPlace {
        backend: TargetId,
        helper: TargetId,
        wrapper: Option<TargetId>,
        antidpi: Option<TargetId>,
        user: String,
        uid: u32,
    },
    Relocate {
        backend: TargetId,
        dest_dir: PathBuf,
        src: PinnedSource,
        dst: PathBuf,
        /// The destination helper's recorded `device:inode`, or `absent`.
        prior_dst: String,
        antidpi: Option<AntidpiRelocate>,
        user: String,
        uid: u32,
    },
}

/// Refuses a root caller outright: enrolling uid 0 in the helper group would
/// grant nothing and hide a misconfigured launch.
pub(super) fn ensure_not_root(uid: u32, euid: u32) -> Result<(), PrivilegeError> {
    if uid == 0 || euid == 0 {
        Err(PrivilegeError::RootEnrollment)
    } else {
        Ok(())
    }
}

/// Resolves the invoking account so the elevated script can pin the exact UID
/// it enrolls. An NSS failure is an error, never a guessed name.
pub(super) fn invoking_account(at: &Path) -> Result<(u32, String), PrivilegeError> {
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
/// until the polkit dialog completes. `antidpi` is the anti-DPI helper to
/// provision in the same elevation, or `None` to leave it untouched — a
/// non-existent path is skipped, so environments that build without the
/// antidpi crate still grant TUN.
pub fn grant(backend: &Path, helper: &Path, antidpi: Option<&Path>) -> Result<(), PrivilegeError> {
    ensure_not_root(
        u32::from(nix::unistd::getuid()),
        u32::from(nix::unistd::geteuid()),
    )?;
    let (uid, user) = invoking_account(backend)?;
    let selection = plan_grant(helper, antidpi)?;

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
        Selection::InPlace {
            helper,
            wrapper,
            antidpi,
        } => {
            let helper_id = validate_mutation_target(&helper)?;
            let wrapper_id = wrapper
                .as_deref()
                .map(validate_mutation_target)
                .transpose()?;
            let antidpi_id = antidpi
                .as_deref()
                .map(validate_mutation_target)
                .transpose()?;
            // A mode repair can enlarge an existing ACL mask and activate
            // dormant named permissions, so all files are inspected before
            // any elevation offers to change them.
            acl_allows_mode_change(&helper_id.path)?;
            if let Some(w) = &wrapper_id {
                acl_allows_mode_change(&w.path)?;
            }
            if let Some(a) = &antidpi_id {
                acl_allows_mode_change(&a.path)?;
            }
            let plan = GrantPlan::InPlace {
                backend: backend_id,
                helper: helper_id,
                wrapper: wrapper_id,
                antidpi: antidpi_id,
                user,
                uid,
            };
            let output = run_elevation(&plan)?;
            if !output.status.success() {
                // The elevation failed partway; report what root provably
                // left behind instead of assuming nothing changed.
                let GrantPlan::InPlace {
                    backend,
                    helper,
                    wrapper,
                    antidpi,
                    user,
                    ..
                } = &plan
                else {
                    unreachable!("plan just constructed as InPlace")
                };
                let backend_caps = required_caps(BACKEND_CAPS);
                let mut probes = vec![StateProbe::new(
                    "route helper",
                    &helper.path,
                    Some(0o750),
                    Some(&HELPER_REQUIRED_CAPS),
                    true,
                )];
                if let Some(w) = wrapper {
                    probes.push(StateProbe::new(
                        "setuid wrapper",
                        &w.path,
                        Some(0o4750),
                        None,
                        false,
                    ));
                }
                if let Some(a) = antidpi {
                    probes.push(StateProbe::new(
                        "anti-DPI helper",
                        &a.path,
                        Some(0o750),
                        Some(&ANTIDPI_REQUIRED_CAPS),
                        true,
                    ));
                }
                probes.push(StateProbe::new(
                    "backend",
                    &backend.path,
                    None,
                    Some(&backend_caps),
                    false,
                ));
                return Err(grant_failed_with_state(
                    &output,
                    confirmed_grant_state(user, &probes),
                ));
            }
            let GrantPlan::InPlace {
                backend,
                helper,
                wrapper,
                antidpi,
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
            if let Some(a) = antidpi {
                verify_grant_target(&a.path, &a.identity, 0o750)?;
                verify_antidpi_caps(&a.path)?;
                verify_enrollment(user, &a.path)?;
            }
            verify_identity(backend)?;
            verify_backend_caps(&backend.path)?;
        }
        Selection::Relocate {
            dest_dir,
            src,
            dst,
            antidpi,
        } => {
            validate_destination(&dest_dir)?;
            let prior_dst = prior_identity(&dst)?;
            let plan = GrantPlan::Relocate {
                backend: backend_id,
                dest_dir,
                src,
                dst,
                prior_dst,
                antidpi,
                user,
                uid,
            };
            let output = run_elevation(&plan)?;
            if !output.status.success() {
                // The elevation failed partway; report what root provably
                // left behind instead of assuming nothing changed.
                let GrantPlan::Relocate {
                    backend,
                    dst,
                    antidpi,
                    user,
                    ..
                } = &plan
                else {
                    unreachable!("plan just constructed as Relocate")
                };
                let backend_caps = required_caps(BACKEND_CAPS);
                let mut probes = vec![StateProbe::new(
                    "relocated route helper",
                    dst,
                    Some(0o750),
                    Some(&HELPER_REQUIRED_CAPS),
                    true,
                )];
                if let Some(a) = antidpi {
                    probes.push(StateProbe::new(
                        "relocated anti-DPI helper",
                        &a.dst,
                        Some(0o750),
                        Some(&ANTIDPI_REQUIRED_CAPS),
                        true,
                    ));
                }
                probes.push(StateProbe::new(
                    "backend",
                    &backend.path,
                    None,
                    Some(&backend_caps),
                    false,
                ));
                return Err(grant_failed_with_state(
                    &output,
                    confirmed_grant_state(user, &probes),
                ));
            }
            let GrantPlan::Relocate {
                backend,
                dst,
                antidpi,
                user,
                ..
            } = &plan
            else {
                unreachable!("plan just constructed as Relocate")
            };
            let stdout = String::from_utf8_lossy(&output.stdout);
            let (installed, installed_antidpi) =
                parse_grant_identities(&stdout).ok_or_else(|| {
                    PrivilegeError::GrantVerification {
                        path: dst.clone(),
                        detail: "elevation output is not a helper-identity record".into(),
                    }
                })?;
            let identity = format!("{}:{}", installed.0, installed.1);
            verify_grant_target(dst, &identity, 0o750)?;
            verify_caps(dst)?;
            verify_enrollment(user, dst)?;
            match (antidpi, installed_antidpi) {
                (Some(target), Some(installed)) => {
                    let identity = format!("{}:{}", installed.0, installed.1);
                    verify_grant_target(&target.dst, &identity, 0o750)?;
                    verify_antidpi_caps(&target.dst)?;
                    verify_enrollment(user, &target.dst)?;
                }
                (None, None) => {}
                _ => {
                    return Err(grant_verification(
                        dst,
                        "anti-DPI identity record missing or unexpected",
                    ));
                }
            }
            verify_identity(backend)?;
            verify_backend_caps(&backend.path)?;
        }
    }
    Ok(())
}

/// Runs the single `pkexec` elevation and captures its output so relocation
/// can verify the identity records the script emits.
///
/// A relocating plan pipes the payload in on stdin rather than having root
/// copy it: an AppImage's squashfs is a FUSE mount with no `allow_other`, and
/// `fuse_allow_current_process()` grants root no exemption, so a `cp` under
/// `pkexec` would fail with EACCES. We are the mounting uid, so we can read
/// it; root only ever writes the destination. The helper's bytes come first
/// and the anti-DPI binary — when the plan carries one — directly after; the
/// elevated script splits the stream by the byte sizes passed as arguments.
/// Opening the sources here also closes the window between validating the
/// paths and reading them.
pub(super) fn run_elevation(plan: &GrantPlan) -> Result<std::process::Output, PrivilegeError> {
    let mut cmd = Command::new("pkexec");
    cmd.args(grant_argv(plan));

    if !matches!(plan, GrantPlan::Relocate { .. }) {
        return cmd.output().map_err(|e| PrivilegeError::Spawn("pkexec", e));
    }

    // Every failure after the spawn kills and reaps the elevated child: a
    // killed or half-written grant must not leave pkexec holding the payload.
    cmd.stdin(Stdio::piped());

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| PrivilegeError::Spawn("pkexec", e))?;
    stream_payloads(&mut child, plan)
}

/// Drives an already-spawned relocating elevation: drains its output before
/// any input is written, streams each pinned source handle in the declared
/// order, verifies every target received its exact pinned length, and on any
/// error terminates and reaps the child so no elevated process survives
/// holding a half-written payload.
pub(super) fn stream_payloads(
    child: &mut std::process::Child,
    plan: &GrantPlan,
) -> Result<std::process::Output, PrivilegeError> {
    let GrantPlan::Relocate { src, antidpi, .. } = plan else {
        unreachable!("stream_payloads is only called for relocating plans")
    };
    let mut stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PrivilegeError::Spawn(
                "pkexec",
                std::io::Error::other("pkexec has no stdin pipe"),
            ));
        }
    };
    // Output drains before any input is written, so pkexec can never
    // deadlock filling a pipe we are not reading yet.
    let stdout_reader = drain_to_end(child.stdout.take());
    let stderr_reader = drain_to_end(child.stderr.take());
    let outcome = stream_pinned(&mut stdin, src).and_then(|()| match antidpi {
        Some(antidpi) => stream_pinned(&mut stdin, &antidpi.src),
        None => Ok(()),
    });
    drop(stdin);
    match outcome {
        Ok(()) => {
            // The elevation has no deadline: the polkit dialog may take
            // arbitrarily long, so both pipes are drained to EOF on their
            // own threads instead of the deadline-bound probe collectors.
            match child.wait() {
                Ok(status) => {
                    let stdout = stdout_reader.join().unwrap_or_default();
                    let stderr = stderr_reader.join().unwrap_or_default();
                    Ok(std::process::Output {
                        status,
                        stdout,
                        stderr,
                    })
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    Err(PrivilegeError::Spawn("pkexec", e))
                }
            }
        }
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            Err(e)
        }
    }
}

/// Streams one pinned source handle into `sink`, sending exactly the pinned
/// length and refusing a short stream — the payload root consumes must be
/// the file that was validated, byte for byte.
pub(super) fn stream_pinned<W: std::io::Write>(
    sink: &mut W,
    source: &PinnedSource,
) -> Result<(), PrivilegeError> {
    let mut limited = (&source.file).take(source.len);
    match std::io::copy(&mut limited, sink) {
        Ok(written) if written == source.len => Ok(()),
        Ok(written) => Err(PrivilegeError::Probe(
            source.path.clone(),
            format!("streamed {written} of {} bytes", source.len),
        )),
        Err(e) => Err(PrivilegeError::Probe(
            source.path.clone(),
            format!("cannot stream payload: {e}"),
        )),
    }
}

/// Reads a pipe to EOF on its own thread, returning the collected bytes. The
/// elevation is unbounded by any deadline, so a plain `read_to_end` is enough;
/// the thread ends once the last holder closes the descriptor.
pub(super) fn drain_to_end<R: Read + Send + 'static>(
    pipe: Option<R>,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    })
}

/// Fails the grant with the elevation's captured stderr; an empty stream gets
/// a placeholder rather than a bare refusal.
pub(super) fn grant_failed(output: &std::process::Output) -> PrivilegeError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    let detail = if detail.is_empty() {
        "no diagnostic output".to_string()
    } else {
        detail.to_string()
    };
    PrivilegeError::GrantFailed(detail)
}

/// One planned target's post-mortem probe. Read-only: a failed elevation is
/// reported as what root provably left in place, never compensated by
/// revoking rights.
pub(super) struct StateProbe<'a> {
    pub(super) label: &'static str,
    pub(super) path: &'a Path,
    pub(super) mode: Option<u32>,
    pub(super) caps: Option<&'a [&'a str]>,
    pub(super) enrolled: bool,
}

impl<'a> StateProbe<'a> {
    pub(super) fn new(
        label: &'static str,
        path: &'a Path,
        mode: Option<u32>,
        caps: Option<&'a [&'a str]>,
        enrolled: bool,
    ) -> Self {
        Self {
            label,
            path,
            mode,
            caps,
            enrolled,
        }
    }

    fn line(&self, user: &str) -> String {
        let mut unconfirmed: Vec<String> = Vec::new();
        match std::fs::symlink_metadata(self.path) {
            Err(e) => unconfirmed.push(format!("cannot stat: {e}")),
            Ok(meta) => {
                if !meta.is_file() {
                    unconfirmed.push("not a regular file".into());
                } else if meta.uid() != 0 {
                    unconfirmed.push("not root-owned".into());
                } else if let Some(mode) = self.mode
                    && meta.mode() & 0o7777 != mode
                {
                    unconfirmed.push(format!(
                        "mode is {:o}, expected {:o}",
                        meta.mode() & 0o7777,
                        mode
                    ));
                }
            }
        }
        if let Some(required) = &self.caps {
            match probe_caps_within(Path::new("getcap"), self.path, required, GETCAP_TIMEOUT) {
                Ok(true) => {}
                Ok(false) => {
                    unconfirmed.push(format!("capabilities missing ({})", required.join(", ")))
                }
                Err(e) => unconfirmed.push(format!("capabilities unverified: {e}")),
            }
        }
        if self.enrolled {
            match account_enrolled(user) {
                Ok(true) => {}
                Ok(false) => unconfirmed.push(format!("not enrolled in {HELPER_GROUP}")),
                Err(e) => unconfirmed.push(format!("enrollment unverified: {e}")),
            }
        }
        if unconfirmed.is_empty() {
            format!("{} {}: provisioned", self.label, self.path.display())
        } else {
            format!(
                "{} {}: not confirmed ({})",
                self.label,
                self.path.display(),
                unconfirmed.join("; ")
            )
        }
    }
}

/// Read-only post-mortem for a failed elevation: per-target confirmed
/// state, including enrollment, so the user learns what actually happened.
pub(super) fn confirmed_grant_state(user: &str, probes: &[StateProbe<'_>]) -> String {
    probes
        .iter()
        .map(|probe| probe.line(user))
        .collect::<Vec<_>>()
        .join("; ")
}

pub(super) fn grant_failed_with_state(
    output: &std::process::Output,
    state: String,
) -> PrivilegeError {
    let PrivilegeError::GrantFailed(detail) = grant_failed(output) else {
        unreachable!("grant_failed always returns GrantFailed")
    };
    PrivilegeError::GrantFailed(format!("{detail}; confirmed state: {state}"))
}

/// Decides between granting in place and relocating, resolving the paths the
/// elevation will touch. `antidpi` is the optional anti-DPI target riding
/// along in the same elevation.
///
/// The copy source comes from `/proc/self/exe` only. Taking it from `$APPDIR`
/// would let anything able to set that variable — a rewritten desktop entry,
/// for instance — choose what root installs.
pub(super) fn plan_grant(
    helper: &Path,
    antidpi: Option<&Path>,
) -> Result<Selection, PrivilegeError> {
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
            antidpi: plan_antidpi_in_place(antidpi)?,
        });
    }

    let src = pinned_source(&bundled)?;
    // The anti-DPI binary relocates only from beside the executable too: root
    // cannot read the AppImage mount, so its bytes travel on stdin beside the
    // route helper's. A build without the antidpi crate simply skips the
    // target.
    let antidpi_relocate = match antidpi {
        Some(_) => match crate::antidpi::bundled_antidpi_path() {
            Some(antidpi_src) => {
                let src = pinned_source(&antidpi_src)?;
                let dst = crate::antidpi::relocated_antidpi_path();
                let prior_dst = prior_identity(&dst)?;
                Some(AntidpiRelocate {
                    src,
                    dst,
                    prior_dst,
                })
            }
            None => None,
        },
        None => None,
    };
    Ok(Selection::Relocate {
        dest_dir: PathBuf::from(crate::tun::RELOCATE_DIR),
        dst: crate::tun::relocated_helper_path(),
        antidpi: antidpi_relocate,
        src,
    })
}

/// Validates the anti-DPI grant target for an in-place plan. A `None` caller
/// argument — or a path that does not exist, or one that cannot be matched to
/// the bundled or relocated copies — skips the target: the anti-DPI helper is
/// optional to the grant, and a copy outside this app's own layout (an
/// install.sh copy, a removed bundled directory) must never block the TUN
/// grant the backend and route helper need.
pub(super) fn plan_antidpi_in_place(
    antidpi: Option<&Path>,
) -> Result<Option<PathBuf>, PrivilegeError> {
    let Some(path) = antidpi.filter(|path| path.exists()) else {
        return Ok(None);
    };
    let Some(bundled_antidpi) = crate::antidpi::bundled_antidpi_path() else {
        return Ok(None);
    };
    let selected = if path.is_absolute() {
        path.to_path_buf()
    } else {
        bundled_antidpi.clone()
    };
    // Exact filename and parent only, mirroring the route helper's acceptance:
    // the bundled copy, the packaged copy beside the executable, or the fixed
    // relocated install.
    let acceptable = paths_equivalent(&selected, &bundled_antidpi)
        || paths_equivalent(&selected, &crate::antidpi::relocated_antidpi_path())
        || (selected.is_absolute()
            && selected
                .file_name()
                .is_some_and(|n| Some(n) == bundled_antidpi.file_name())
            && (selected.parent() == bundled_antidpi.parent()
                || selected.parent() == Some(std::path::Path::new(crate::tun::RELOCATE_DIR))));
    if !acceptable {
        return Ok(None);
    }
    Ok(Some(selected))
}

pub(super) fn paths_equivalent(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// True exactly when the grant planner would provision the optional anti-DPI
/// helper at `candidate`: the path exists and matches the bundled or the
/// relocated copy. Absent candidates, PATH-only copies, and unmanaged
/// installs return false — the TUN grant must not depend on them.
pub fn antidpi_grant_eligible(candidate: Option<&Path>) -> bool {
    if crate::tun::relocation_required() {
        // The relocating planner takes the bundled copy whenever it exists;
        // the candidate path itself is not consulted.
        candidate.is_some() && crate::antidpi::bundled_antidpi_path().is_some()
    } else {
        plan_antidpi_in_place(candidate).ok().flatten().is_some()
    }
}

/// Validates an operator-recovery target before any elevation is offered:
/// expected basename, packaged or relocated location, root-owned regular
/// non-symlink file, no non-root write bits, protected parents. A refusal
/// here never reaches `pkexec`.
pub(crate) fn validate_recovery_target(path: &Path) -> Result<PathBuf, PrivilegeError> {
    validate_recovery_target_at(
        path,
        crate::tun::bundled_helper_path().as_deref(),
        &crate::tun::relocated_helper_path(),
    )
}

/// The recovery-target check with the expected copies injected, so tests can
/// name their own packaged and relocated locations. Success yields the
/// canonical path: the caller must execute exactly what was validated, not
/// the raw caller-supplied name that could be re-resolved differently.
pub(super) fn validate_recovery_target_at(
    path: &Path,
    bundled: Option<&Path>,
    relocated: &Path,
) -> Result<PathBuf, PrivilegeError> {
    let not_helper = || unsafe_target(path, "recovery target is not a non-symlink regular file");
    let lstat = std::fs::symlink_metadata(path).map_err(|_| not_helper())?;
    let canonical = std::fs::canonicalize(path).map_err(|_| not_helper())?;
    let meta = std::fs::metadata(&canonical).map_err(|_| not_helper())?;
    if let Some(detail) = recovery_location_violation(&canonical, bundled, relocated) {
        return Err(unsafe_target(path, &detail));
    }
    if lstat.file_type().is_symlink() {
        return Err(unsafe_target(
            path,
            "target is not a non-symlink regular file",
        ));
    }
    require_protected_parents(&canonical)?;
    if let Some(detail) = file_target_violation(
        lstat.file_type().is_symlink(),
        meta.is_file(),
        meta.uid(),
        meta.mode(),
    ) {
        return Err(unsafe_target(path, detail));
    }
    Ok(canonical)
}

/// Location-only classifier for a recovery target: the exact helper basename
/// and one of the two managed copies. Path resolution is not authorization,
/// but recovery must never elevate an arbitrary file that merely shares the
/// helper's name.
pub(super) fn recovery_location_violation(
    canonical: &Path,
    bundled: Option<&Path>,
    relocated: &Path,
) -> Option<String> {
    if canonical.file_name() != Some(std::ffi::OsStr::new(crate::tun::HELPER_BIN)) {
        return Some(format!(
            "recovery target must be named {}",
            crate::tun::HELPER_BIN
        ));
    }
    let known = bundled.is_some_and(|bundled| paths_equivalent(canonical, bundled))
        || paths_equivalent(canonical, relocated);
    if known {
        None
    } else {
        Some("recovery target is neither the packaged nor the relocated route helper".to_string())
    }
}

/// Pure classifier for one mutation target's metadata. The detail strings are
/// user-visible refusal texts and must not drift from the elevated script.
pub(super) fn file_target_violation(
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

pub(super) fn unsafe_target(path: &Path, detail: &str) -> PrivilegeError {
    PrivilegeError::UnsafeGrantTarget {
        path: path.to_path_buf(),
        detail: detail.to_string(),
    }
}

/// Validates one existing mutation target and records its canonical path and
/// identity. Never chown a user-owned or writable file: a descriptor opened
/// before the change could survive it.
pub(super) fn validate_mutation_target(path: &Path) -> Result<TargetId, PrivilegeError> {
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
pub(super) fn require_protected_parents(canonical: &Path) -> Result<(), PrivilegeError> {
    for parent in canonical.ancestors().skip(1) {
        if parent.as_os_str().is_empty() {
            break;
        }
        check_protected_dir(parent)?;
    }
    Ok(())
}

pub(super) fn check_protected_dir(dir: &Path) -> Result<(), PrivilegeError> {
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
pub(super) fn validate_destination(dest_dir: &Path) -> Result<(), PrivilegeError> {
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
pub(super) fn prior_identity(dst: &Path) -> Result<String, PrivilegeError> {
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

/// trusted bundle: a read-only mount or a protected root-owned directory
/// chain. An arbitrary user-writable `nosuid` directory is neither — its
/// contents are caller-controlled, and elevating them would install
/// whatever the invoking user staged there.
pub(super) fn pinned_source(src: &Path) -> Result<PinnedSource, PrivilegeError> {
    let refuse = |why: String| PrivilegeError::Probe(src.to_path_buf(), why);

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .and_then(|d| std::fs::canonicalize(d).ok())
        .ok_or_else(|| refuse("cannot resolve the executable's directory".into()))?;
    // Stat before canonicalizing: canonicalize resolves the final component
    // too, so a symlink test on its result could never be true.
    let lstat = std::fs::symlink_metadata(src).map_err(|e| refuse(format!("cannot stat: {e}")))?;
    if lstat.file_type().is_symlink() {
        return Err(refuse("source is a symlink".into()));
    }
    let mut options = std::fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(nix::fcntl::OFlag::O_NOFOLLOW.bits());
    let file = options
        .open(src)
        .map_err(|e| refuse(format!("cannot open: {e}")))?;
    let meta = file
        .metadata()
        .map_err(|e| refuse(format!("cannot stat the open file: {e}")))?;
    if !meta.is_file() {
        return Err(refuse("not a regular file".into()));
    }
    let canonical =
        std::fs::canonicalize(src).map_err(|e| refuse(format!("cannot resolve: {e}")))?;
    if !source_acceptable(&canonical, &exe_dir, false, true) {
        return Err(refuse("not a regular file beside the executable".into()));
    }
    source_bundle_trusted(&canonical)?;
    Ok(PinnedSource {
        file,
        path: canonical,
        identity: format!("{}:{}", meta.dev(), meta.ino()),
        len: meta.len(),
    })
}

pub(super) fn source_acceptable(
    canonical_src: &Path,
    exe_dir: &Path,
    is_symlink: bool,
    is_regular: bool,
) -> bool {
    !is_symlink && is_regular && canonical_src.starts_with(exe_dir)
}

/// A relocation source is trusted only where its location cannot have been
/// staged by the invoking user: a read-only mount (an AppImage's squashfs)
/// or a protected root-owned directory chain.
pub(super) fn source_bundle_trusted(canonical: &Path) -> Result<(), PrivilegeError> {
    if mount_opts_readonly(canonical) {
        return Ok(());
    }
    require_protected_parents(canonical)
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
pub(super) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Builds the `pkexec` argument vector. Every binary path is passed as a
/// positional parameter so the shell never parses it as code — a user-
/// controlled path containing shell metacharacters cannot inject commands.
///
/// Positionals above nine must be braced: `$10` would be read as `$1`
/// followed by a literal `0`.
pub(super) fn grant_argv(plan: &GrantPlan) -> Vec<OsString> {
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
            antidpi,
            user,
            uid,
        } => {
            // $1 BACKEND_CAPS, $2 backend, $3 HELPER_CAPS, $4 helper,
            // $5 HELPER_GROUP, $6 account, $7 wrapper or empty,
            // $8 backend identity, $9 helper identity,
            // ${10} wrapper identity or empty, ${11} invoking UID,
            // ${12} anti-DPI path or empty, ${13} anti-DPI caps or empty,
            // ${14} anti-DPI identity or empty.
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
            argv.push(
                antidpi
                    .as_ref()
                    .map(|a| a.path.as_os_str().to_os_string())
                    .unwrap_or_default(),
            );
            argv.push(match antidpi {
                Some(_) => OsString::from(ANTIDPI_CAPS),
                None => OsString::default(),
            });
            argv.push(
                antidpi
                    .as_ref()
                    .map(|a| OsString::from(&a.identity))
                    .unwrap_or_default(),
            );
        }
        GrantPlan::Relocate {
            backend,
            dest_dir,
            dst,
            user,
            uid,
            prior_dst,
            antidpi,
            src,
        } => {
            // $1 BACKEND_CAPS, $2 backend, $3 dest dir, $4 HELPER_GROUP,
            // $5 destination, $6 HELPER_CAPS, $7 account,
            // $8 backend identity, $9 invoking UID,
            // ${10} prior destination identity or `absent`,
            // ${11} anti-DPI destination or empty, ${12} anti-DPI caps or empty,
            // ${13} helper payload size in bytes, ${14} anti-DPI payload size or empty,
            // ${15} prior anti-DPI destination identity or `absent`.
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
            argv.push(
                antidpi
                    .as_ref()
                    .map(|a| a.dst.as_os_str().to_os_string())
                    .unwrap_or_default(),
            );
            argv.push(match antidpi {
                Some(_) => OsString::from(ANTIDPI_CAPS),
                None => OsString::default(),
            });
            argv.push(OsString::from(src.len.to_string()));
            argv.push(match antidpi {
                Some(a) => OsString::from(a.src.len.to_string()),
                None => OsString::default(),
            });
            argv.push(match antidpi {
                Some(a) => OsString::from(&a.prior_dst),
                None => OsString::default(),
            });
        }
    }
    argv
}

pub(super) fn grant_script(plan: &GrantPlan) -> &'static str {
    match plan {
        // Every target is re-validated under elevation before the first
        // mutation: non-dereferencing stat, exact ownership and mode bits,
        // the recorded device:inode identity, and protected parents, so
        // nothing can be swapped across the polkit prompt. Ownership changes
        // precede modes and capability grants because chown clears both file
        // capabilities and the setuid bit, and the wrapper's setuid bit is
        // applied last so a late capability failure can never leave a setuid
        // binary without the caps it needs.
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
             if [ -n \"${12}\" ]; then check_file \"${12}\" \"${14}\"; check_parents \"${12}\"; check_acl \"${12}\"; fi\n\
             check_acl \"$4\"\n\
             [ -z \"$7\" ] || check_acl \"$7\"\n\
             if getent group \"$5\" >/dev/null 2>&1; then :\n\
             else\n\
               rc=$?\n\
               [ \"$rc\" = 2 ] || refuse \"group lookup failed: $5\"\n\
               groupadd --system \"$5\" || refuse \"group creation failed: $5\"\n\
             fi\n\
             if [ -n \"$7\" ]; then\n\
               if getent passwd v2ray-rs-bypass >/dev/null 2>&1; then :\n\
               else\n\
                 rc=$?\n\
                 [ \"$rc\" = 2 ] || refuse \"account lookup failed: v2ray-rs-bypass\"\n\
                 useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin v2ray-rs-bypass || refuse \"bypass account creation failed\"\n\
               fi\n\
             fi\n\
             usermod -aG \"$5\" -- \"$6\" || refuse \"enrollment failed: $6\"\n\
             chown \"root:$5\" \"$4\" || refuse \"ownership change failed: $4\"\n\
             chmod 0750 \"$4\" || refuse \"mode change failed: $4\"\n\
             setcap \"$3\" \"$4\" || refuse \"capability grant failed: $4\"\n\
             if [ -n \"$7\" ]; then\n\
               chown \"root:$5\" \"$7\" || refuse \"ownership change failed: $7\"\n\
             fi\n\
             if [ -n \"${12}\" ]; then\n\
               chown \"root:$5\" \"${12}\" || refuse \"ownership change failed: ${12}\"\n\
               chmod 0750 \"${12}\" || refuse \"mode change failed: ${12}\"\n\
               setcap \"${13}\" \"${12}\" || refuse \"capability grant failed: ${12}\"\n\
             fi\n\
             setcap \"$1\" \"$2\" || refuse \"capability grant failed: $2\"\n\
             if [ -n \"$7\" ]; then\n\
               chmod 4750 \"$7\" || refuse \"mode change failed: $7\"\n\
             fi\n"
        }
        // The payload arrives on stdin, from a descriptor the unprivileged
        // caller opened and pinned: root cannot read an AppImage's FUSE
        // mount at all. Every publisher holds a root-only flock on
        // /run/v2ray-rs-grant.lock from recovery through final cleanup, so
        // concurrent grants serialize and the later one re-checks both prior
        // identities against the winner's result. Payloads stage in a
        // mode-0700 transaction directory on the destination filesystem,
        // consumed by the exact declared byte count, attributed, and
        // re-read back — mode with `stat`, capability set with `getcap` —
        // before anything is published; the fallible backend `setcap` also
        // precedes the first rename. Prior absence and staged identities
        // are recorded in a state file before publishing; existing
        // destinations are hard-linked into the transaction directory
        // without ever unlinking their live names, so the prior inode stays
        // alive until both publications and identities verify. On failure
        // the original is restored over the new file, or a first install is
        // removed by its verified identity; a destination matching neither
        // record is preserved and reported as a conflict, never swept. A
        // grant killed mid-publication leaves the state file behind, and
        // the next grant recovers it under the same lock before re-checking
        // the priors. Publishing is an atomic same-directory `mv -f`;
        // nothing ever truncates or unlinks a destination. `chown` precedes
        // `chmod`/`setcap` because it clears both file capabilities and the
        // setuid bit. The group is a dedicated system group rather than the
        // caller's primary group, which on distributions that share one
        // (gid 100 `users`) would hand every local account a cap_net_admin
        // binary. Both destinations are written before any receipt is
        // printed: the helper record first, the optional anti-DPI record
        // last, so the unprivileged caller can verify the whole grant from
        // the captured stdout.
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
             check_prior() {\n\
               if [ -e \"$1\" ] || [ -L \"$1\" ]; then\n\
                 [ \"$2\" != absent ] || refuse \"target identity changed: $1\"\n\
                 check_file \"$1\" \"$2\"\n\
               else\n\
                 [ \"$2\" = absent ] || refuse \"target identity changed: $1\"\n\
               fi\n\
             }\n\
             caps_cover() {\n\
               for clause in $1; do\n\
                 case \"$clause\" in\n\
                   *=*) names=${clause%%=*}; flags=${clause#*=} ;;\n\
                   *+*) names=${clause%%+*}; flags=${clause#*+} ;;\n\
                   *) return 1 ;;\n\
                 esac\n\
                 case \"$flags\" in *[!eip]*) return 1 ;; esac\n\
                 case \"$flags\" in *e*p*|*p*e*) ;; *) continue ;; esac\n\
                 [ -n \"$names\" ] || return 0\n\
                 case \",$names,\" in *\",$2,\"*) return 0 ;; esac\n\
               done\n\
               return 1\n\
             }\n\
             check_staged() {\n\
               [ \"$(stat -c '%a' \"$1\")\" = 750 ] || refuse \"staged mode verification failed: $2\"\n\
               staged_caps=$(LC_ALL=C getcap \"$1\" 2>/dev/null) || refuse \"staged capability verification failed: $2\"\n\
               case \"$staged_caps\" in\n\
                 \"$1 \"*) staged_caps=${staged_caps#\"$1 \"} ;;\n\
                 *) refuse \"staged capability verification failed: $2\" ;;\n\
               esac\n\
               want_names=${3%%[+=]*}\n\
               want_flags=${3#*[+=]}\n\
               [ -n \"$want_names\" ] && [ -n \"$want_flags\" ] || refuse \"staged capability verification failed: $2\"\n\
               rest=$want_names\n\
               while [ -n \"$rest\" ]; do\n\
                 case \"$rest\" in\n\
                   *,*)\n\
                     want=${rest%%,*}\n\
                     rest=${rest#*,} ;;\n\
                   *)\n\
                     want=$rest\n\
                     rest= ;;\n\
                 esac\n\
                 caps_cover \"$staged_caps\" \"$want\" || refuse \"staged capability verification failed: $2\"\n\
               done\n\
             }\n\
             conflict() {\n\
               echo \"refusing TUN privilege grant: interrupted transaction conflict: $1\" >&2\n\
               exit 1\n\
             }\n\
             recover_target() {\n\
               cur=$(stat -c '%d:%i' \"$1\" 2>/dev/null || echo absent)\n\
               if [ -n \"$4\" ]; then\n\
                 [ \"$2\" != absent ] || conflict \"$1 is backed up but the transaction recorded it absent\"\n\
                 case \"$cur\" in\n\
                   \"$2\"|\"$3\"|absent) mv -f \"$4\" \"$1\" || conflict \"cannot restore $1\" ;;\n\
                   *) conflict \"$1 changed during an interrupted grant; left untouched\" ;;\n\
                 esac\n\
               elif [ \"$2\" != absent ]; then\n\
                 [ \"$cur\" = \"$2\" ] || conflict \"$1 changed during an interrupted grant; left untouched\"\n\
               elif [ \"$cur\" = absent ]; then\n\
                 :\n\
               elif [ \"$3\" != absent ] && [ \"$cur\" = \"$3\" ]; then\n\
                 rm -f \"$1\" || conflict \"cannot remove the interrupted grant's $1\"\n\
               else\n\
                 conflict \"$1 changed during an interrupted grant; left untouched\"\n\
               fi\n\
             }\n\
             recover_interrupted() {\n\
               [ -f \"$STATE\" ] || return 0\n\
               tx=\n\
               rec_hd=\n\
               rec_hp=\n\
               rec_hn=\n\
               rec_ad=\n\
               rec_ap=\n\
               rec_an=\n\
               while IFS= read -r line || [ -n \"$line\" ]; do\n\
                 case \"$line\" in\n\
                   tx=*) tx=${line#tx=} ;;\n\
                   helper-dst=*) rec_hd=${line#helper-dst=} ;;\n\
                   helper-prior=*) rec_hp=${line#helper-prior=} ;;\n\
                   helper-new=*) rec_hn=${line#helper-new=} ;;\n\
                   antidpi-dst=*) rec_ad=${line#antidpi-dst=} ;;\n\
                   antidpi-prior=*) rec_ap=${line#antidpi-prior=} ;;\n\
                   antidpi-new=*) rec_an=${line#antidpi-new=} ;;\n\
                   *-len=*|*-mode=*) ;;\n\
                   *) conflict \"unrecognized record in $STATE\" ;;\n\
                 esac\n\
               done < \"$STATE\"\n\
               [ -n \"$tx\" ] || conflict \"transaction record has no transaction directory\"\n\
               case \"$tx\" in\n\
                 \"$dest_dir\"/.v2ray-rs-tx.*) [ -d \"$tx\" ] || conflict \"transaction record is not a directory\" ;;\n\
                 *) conflict \"transaction record points outside $dest_dir\" ;;\n\
               esac\n\
               rec_hb=$tx/helper-backup\n\
               [ -e \"$rec_hb\" ] || rec_hb=\n\
               recover_target \"$rec_hd\" \"$rec_hp\" \"$rec_hn\" \"${rec_hb:-}\"\n\
               if [ -n \"$rec_ad\" ]; then\n\
                 rec_ab=$tx/antidpi-backup\n\
                 [ -e \"$rec_ab\" ] || rec_ab=\n\
                 recover_target \"$rec_ad\" \"$rec_ap\" \"$rec_an\" \"${rec_ab:-}\"\n\
               fi\n\
               rm -f \"$STATE\"\n\
               rm -rf \"$tx\"\n\
             }\n\
             publish_failed() {\n\
               detail=$1\n\
               if [ -n \"$rb_h\" ]; then\n\
                 cur=$(stat -c '%d:%i' \"$dst_h\" 2>/dev/null || echo absent)\n\
                 case \"$cur\" in\n\
                   \"$new_h\"|\"$prior_h\"|absent)\n\
                     if mv -f \"$rb_h\" \"$dst_h\"; then\n\
                       rb_h=\n\
                     else\n\
                       detail=\"$detail; original route helper preserved at $rb_h\"\n\
                     fi ;;\n\
                   *)\n\
                     detail=\"$detail; route helper destination changed during the grant, left at $dst_h\" ;;\n\
                 esac\n\
               elif [ \"$prior_h\" = absent ] && [ -e \"$dst_h\" ]; then\n\
                 cur=$(stat -c '%d:%i' \"$dst_h\" 2>/dev/null || echo absent)\n\
                 if [ \"$cur\" = \"$new_h\" ]; then\n\
                   rm -f \"$dst_h\" || detail=\"$detail; could not remove the partially published route helper\"\n\
                 else\n\
                   detail=\"$detail; route helper destination changed during the grant, left at $dst_h\"\n\
                 fi\n\
               fi\n\
               if [ -n \"$rb_a\" ]; then\n\
                 cur=$(stat -c '%d:%i' \"$dst_a\" 2>/dev/null || echo absent)\n\
                 case \"$cur\" in\n\
                   \"$new_a\"|\"$prior_a\"|absent)\n\
                     if mv -f \"$rb_a\" \"$dst_a\"; then\n\
                       rb_a=\n\
                     else\n\
                       detail=\"$detail; original anti-DPI helper preserved at $rb_a\"\n\
                     fi ;;\n\
                   *)\n\
                     detail=\"$detail; anti-DPI destination changed during the grant, left at $dst_a\" ;;\n\
                 esac\n\
               elif [ -n \"$dst_a\" ] && [ \"$prior_a\" = absent ] && [ -e \"$dst_a\" ]; then\n\
                 cur=$(stat -c '%d:%i' \"$dst_a\" 2>/dev/null || echo absent)\n\
                 if [ \"$cur\" = \"$new_a\" ]; then\n\
                   rm -f \"$dst_a\" || detail=\"$detail; could not remove the partially published anti-DPI helper\"\n\
                 else\n\
                   detail=\"$detail; anti-DPI destination changed during the grant, left at $dst_a\"\n\
                 fi\n\
               fi\n\
               if [ -z \"$rb_h\" ] && [ -z \"$rb_a\" ]; then\n\
                 rm -f \"$STATE\" 2>/dev/null || :\n\
                 rm -rf \"$txdir\" 2>/dev/null || :\n\
               fi\n\
               refuse \"$detail\"\n\
             }\n\
             cleanup() {\n\
               if [ -f \"$STATE\" ]; then\n\
                 :\n\
               else\n\
                 [ -z \"$txdir\" ] || rm -rf \"$txdir\" 2>/dev/null || :\n\
               fi\n\
             }\n\
             check_file \"$2\" \"$8\"; check_parents \"$2\"\n\
             GRANT_LOCK=/run/v2ray-rs-grant.lock\n\
             [ -d /run ] || refuse \"grant lock unavailable: /run is missing\"\n\
             ( umask 077; : > \"$GRANT_LOCK\" ) 2>/dev/null || :\n\
             check_lock() {\n\
               [ ! -L \"$GRANT_LOCK\" ] && [ -f \"$GRANT_LOCK\" ] || refuse \"grant lock is not a regular file: $GRANT_LOCK\"\n\
               [ \"$(stat -c '%h' \"$GRANT_LOCK\")\" = 1 ] || refuse \"grant lock is not single-linked: $GRANT_LOCK\"\n\
               [ \"$(stat -c '%u' \"$GRANT_LOCK\")\" = 0 ] || refuse \"grant lock is not root-owned: $GRANT_LOCK\"\n\
               [ \"$(stat -c '%a' \"$GRANT_LOCK\")\" = 600 ] || refuse \"grant lock is not owner-only: $GRANT_LOCK\"\n\
             }\n\
             check_lock\n\
             exec 9>>\"$GRANT_LOCK\" || refuse \"grant lock unavailable: $GRANT_LOCK\"\n\
             case \"$(readlink /proc/self/fd/9)\" in \"$GRANT_LOCK\") ;; *) refuse \"grant lock was swapped: $GRANT_LOCK\" ;; esac\n\
             flock 9 || refuse \"grant lock failed\"\n\
             [ \"$(id -u \"$7\")\" = \"$9\" ] || refuse \"invoking account changed\"\n\
             [ \"$9\" != 0 ] || refuse \"cannot enroll uid 0\"\n\
             dst_h=$5\n\
             dst_a=${11}\n\
             prior_h=${10}\n\
             prior_a=${15}\n\
             dest_dir=$3\n\
             STATE=$dest_dir/.v2ray-rs-grant-state\n\
             txdir=\n\
             tmp_h=\n\
             tmp_a=\n\
             rb_h=\n\
             rb_a=\n\
             new_h=\n\
             new_a=\n\
             if [ -e \"$3\" ]; then\n\
               check_dir \"$3\"; check_parents \"$3\"\n\
             else\n\
               p=\"$3\"\n\
               while [ ! -e \"$p\" ]; do p=$(dirname -- \"$p\"); done\n\
               check_dir \"$p\"; check_parents \"$p\"\n\
               mkdir -p \"$3\" || refuse \"destination creation failed: $3\"\n\
             fi\n\
             recover_interrupted\n\
             check_prior \"$5\" \"${10}\"; check_parents \"$5\"\n\
             if [ -n \"${11}\" ]; then\n\
               check_prior \"${11}\" \"${15}\"; check_parents \"${11}\"\n\
             fi\n\
             if getent group \"$4\" >/dev/null 2>&1; then :\n\
             else\n\
               rc=$?\n\
               [ \"$rc\" = 2 ] || refuse \"group lookup failed: $4\"\n\
               groupadd --system \"$4\" || refuse \"group creation failed: $4\"\n\
             fi\n\
             usermod -aG \"$4\" -- \"$7\" || refuse \"enrollment failed: $7\"\n\
             trap 'exit 1' HUP INT TERM\n\
             trap cleanup EXIT\n\
             txdir=$(mktemp -d \"$3/.v2ray-rs-tx.XXXXXX\") || refuse \"transaction directory creation failed: $3\"\n\
             tmp_h=$txdir/helper-staged\n\
             head -c \"${13}\" > \"$tmp_h\" || refuse \"payload copy failed: $5\"\n\
             [ \"$(stat -c %s \"$tmp_h\")\" = \"${13}\" ] || refuse \"payload size mismatch: $5\"\n\
             if [ -n \"${11}\" ]; then\n\
               tmp_a=$txdir/antidpi-staged\n\
               head -c \"${14}\" > \"$tmp_a\" || refuse \"payload copy failed: ${11}\"\n\
               [ \"$(stat -c %s \"$tmp_a\")\" = \"${14}\" ] || refuse \"payload size mismatch: ${11}\"\n\
             fi\n\
             [ \"$(head -c 1 | wc -c)\" = 0 ] || refuse \"unexpected excess payload\"\n\
             chown \"root:$4\" \"$tmp_h\" || refuse \"ownership change failed: $5\"\n\
             chmod 0750 \"$tmp_h\" || refuse \"mode change failed: $5\"\n\
             setcap \"$6\" \"$tmp_h\" || refuse \"capability grant failed: $5\"\n\
             if [ -n \"${11}\" ]; then\n\
               chown \"root:$4\" \"$tmp_a\" || refuse \"ownership change failed: ${11}\"\n\
               chmod 0750 \"$tmp_a\" || refuse \"mode change failed: ${11}\"\n\
               setcap \"${12}\" \"$tmp_a\" || refuse \"capability grant failed: ${11}\"\n\
             fi\n\
             check_staged \"$tmp_h\" \"$5\" \"$6\"\n\
             if [ -n \"${11}\" ]; then check_staged \"$tmp_a\" \"${11}\" \"${12}\"; fi\n\
             setcap \"$1\" \"$2\" || refuse \"capability grant failed: $2\"\n\
             new_h=$(stat -c '%d:%i' \"$tmp_h\")\n\
             if [ -n \"${11}\" ]; then new_a=$(stat -c '%d:%i' \"$tmp_a\"); fi\n\
             {\n\
               printf 'tx=%s\\n' \"$txdir\"\n\
               printf 'helper-dst=%s\\n' \"$dst_h\"\n\
               printf 'helper-prior=%s\\n' \"$prior_h\"\n\
               printf 'helper-new=%s\\n' \"$new_h\"\n\
               printf 'helper-len=%s\\n' \"$(stat -c %s \"$tmp_h\")\"\n\
               printf 'helper-mode=%s\\n' \"$(stat -c '%a' \"$tmp_h\")\"\n\
               if [ -n \"${11}\" ]; then\n\
                 printf 'antidpi-dst=%s\\n' \"$dst_a\"\n\
                 printf 'antidpi-prior=%s\\n' \"$prior_a\"\n\
                 printf 'antidpi-new=%s\\n' \"$new_a\"\n\
                 printf 'antidpi-len=%s\\n' \"$(stat -c %s \"$tmp_a\")\"\n\
                 printf 'antidpi-mode=%s\\n' \"$(stat -c '%a' \"$tmp_a\")\"\n\
               fi\n\
             } > \"$STATE\"\n\
             backup_h=$txdir/helper-backup\n\
             backup_a=$txdir/antidpi-backup\n\
             if [ \"$prior_h\" != absent ]; then\n\
               ln \"$dst_h\" \"$backup_h\" || refuse \"backup creation failed: $dst_h\"\n\
               rb_h=$backup_h\n\
             fi\n\
             if [ -n \"${11}\" ] && [ \"$prior_a\" != absent ]; then\n\
               ln \"$dst_a\" \"$backup_a\" || refuse \"backup creation failed: $dst_a\"\n\
               rb_a=$backup_a\n\
             fi\n\
             if mv -f \"$tmp_h\" \"$dst_h\"; then\n\
               tmp_h=\n\
             else\n\
               publish_failed \"destination publish failed: $dst_h\"\n\
             fi\n\
             if [ -n \"${11}\" ]; then\n\
               if mv -f \"$tmp_a\" \"$dst_a\"; then\n\
                 tmp_a=\n\
               else\n\
                 publish_failed \"destination publish failed: $dst_a\"\n\
               fi\n\
             fi\n\
             [ \"$(stat -c '%d:%i' \"$dst_h\")\" = \"$new_h\" ] || publish_failed \"published identity mismatch: $dst_h\"\n\
             if [ -n \"${11}\" ]; then\n\
               [ \"$(stat -c '%d:%i' \"$dst_a\")\" = \"$new_a\" ] || publish_failed \"published identity mismatch: $dst_a\"\n\
             fi\n\
             rm -f \"$STATE\"\n\
             rm -rf \"$txdir\"\n\
             printf 'helper-identity=%s\\n' \"$new_h\"\n\
             if [ -n \"${11}\" ]; then\n\
               printf 'antidpi-identity=%s\\n' \"$new_a\"\n\
             fi\n"
        }
    }
}

/// Ordered paths the grant elevation needs capability support on, each paired
/// with the caps a manual `setcap` would apply.
///
/// A relocating plan checks the destination directory, not the bundled copy: the
/// bundled copy's mount is known to be `nosuid`, which is the whole reason for
/// relocating.
pub(super) fn preflight_targets<'a>(
    backend: &'a Path,
    plan: &'a Selection,
) -> Vec<(&'a Path, &'static str)> {
    let mut targets = vec![(backend, BACKEND_CAPS)];
    match plan {
        Selection::InPlace {
            helper,
            wrapper,
            antidpi,
        } => {
            targets.push((helper.as_path(), HELPER_CAPS));
            if let Some(wp) = wrapper {
                targets.push((wp.as_path(), HELPER_CAPS));
            }
            if let Some(ap) = antidpi {
                targets.push((ap.as_path(), ANTIDPI_CAPS));
            }
        }
        Selection::Relocate {
            dest_dir, antidpi, ..
        } => {
            targets.push((dest_dir.as_path(), HELPER_CAPS));
            if let Some(a) = antidpi {
                targets.push((a.dst.as_path(), ANTIDPI_CAPS));
            }
        }
    }
    targets
}
