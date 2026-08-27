use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    #[error("read capabilities of {0}: {1}")]
    Probe(PathBuf, String),
    #[error(
        "{path} is on a filesystem that ignores file capabilities (e.g. mounted nosuid). \
         Grant manually after moving the binary, or run: sudo setcap '{caps}' {path}"
    )]
    Unsupported { path: PathBuf, caps: String },
    #[error(
        "cannot install the TUN helper into {dest}: that filesystem ignores file \
         capabilities (mounted nosuid). Install the distribution package instead."
    )]
    UnsupportedDest { dest: PathBuf },
    #[error("the TUN helper was not installed at {0}; the privileged step did not complete")]
    RelocateFailed(PathBuf),
    #[error("run {0}: {1}")]
    Spawn(&'static str, std::io::Error),
    #[error("privilege grant was cancelled or failed")]
    GrantFailed,
}

/// Reports whether the binary at `path` already holds `cap_net_admin` in its
/// file capabilities.
pub fn has_net_admin(path: &Path) -> Result<bool, PrivilegeError> {
    let output = Command::new("getcap")
        .arg(path)
        .output()
        .map_err(|e| PrivilegeError::Spawn("getcap", e))?;

    if !output.status.success() {
        return Err(PrivilegeError::Probe(
            path.to_path_buf(),
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    Ok(getcap_has_cap(
        &String::from_utf8_lossy(&output.stdout),
        "cap_net_admin",
    ))
}

/// What a single `pkexec` elevation has to do.
///
/// `InPlace` grants capabilities where the binaries already live. `Relocate`
/// first copies the route helper somewhere it can hold them, which is the only
/// option when the bundled copy sits on a `nosuid` mount.
#[derive(Debug, PartialEq, Eq)]
enum GrantPlan {
    InPlace {
        helper: PathBuf,
        wrapper: Option<PathBuf>,
    },
    Relocate {
        dest_dir: PathBuf,
        src: PathBuf,
        dst: PathBuf,
        user: String,
    },
}

/// Grants the required capabilities via a single `pkexec` elevation. Blocks
/// until the polkit dialog completes.
pub fn grant(backend: &Path, helper: &Path) -> Result<(), PrivilegeError> {
    let plan = plan_grant(helper)?;

    // setcap and chmod u+s exit 0 on a nosuid mount, so flag the offending path
    // and its remedy before prompting for elevation rather than after.
    for (path, caps) in preflight_targets(backend, &plan) {
        if !file_caps_supported(path) {
            return Err(match &plan {
                GrantPlan::Relocate { dest_dir, .. } if path == dest_dir => {
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

    let status = run_elevation(backend, &plan)?;
    if !status.success() {
        return Err(PrivilegeError::GrantFailed);
    }

    // A relocating grant writes a new root-owned binary; confirm it actually
    // landed and carries the capability rather than trusting the exit status.
    if let GrantPlan::Relocate { dst, .. } = &plan {
        let landed = std::fs::symlink_metadata(dst).is_ok_and(|m| m.is_file());
        // `getcap` may be absent from the user's PATH even though the elevation
        // ran `setcap` fine, so only a definite "no capability" is a failure.
        let capped = has_net_admin(dst).unwrap_or(true);
        if !landed || !capped {
            return Err(PrivilegeError::RelocateFailed(dst.clone()));
        }
    }
    Ok(())
}

/// Runs the single `pkexec` elevation.
///
/// A relocating plan pipes the helper in on stdin rather than having root copy
/// it: an AppImage's squashfs is a FUSE mount with no `allow_other`, and
/// `fuse_allow_current_process()` grants root no exemption, so a `cp` under
/// `pkexec` would fail with EACCES. We are the mounting uid, so we can read it;
/// root only ever writes the destination. Opening the source here also closes
/// the window between validating the path and reading it.
fn run_elevation(
    backend: &Path,
    plan: &GrantPlan,
) -> Result<std::process::ExitStatus, PrivilegeError> {
    let mut cmd = Command::new("pkexec");
    cmd.args(grant_argv(backend, plan));

    let GrantPlan::Relocate { src, .. } = plan else {
        return cmd.status().map_err(|e| PrivilegeError::Spawn("pkexec", e));
    };

    let payload = std::fs::File::open(src)
        .map_err(|e| PrivilegeError::Probe(src.clone(), format!("cannot read: {e}")))?;
    cmd.stdin(payload);
    cmd.status().map_err(|e| PrivilegeError::Spawn("pkexec", e))
}

/// Decides between granting in place and relocating, resolving and validating
/// every path the elevation will touch.
///
/// The copy source comes from `/proc/self/exe` only. Taking it from `$APPDIR`
/// would let anything able to set that variable — a rewritten desktop entry, for
/// instance — choose what root installs.
fn plan_grant(helper: &Path) -> Result<GrantPlan, PrivilegeError> {
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
        let helper = if helper.is_absolute() {
            helper.to_path_buf()
        } else {
            bundled
        };
        return Ok(GrantPlan::InPlace {
            helper,
            wrapper: crate::tun::bundled_run_path(),
        });
    }

    let src = validated_source(&bundled)?;
    let user = nix::unistd::User::from_uid(nix::unistd::getuid())
        .ok()
        .flatten()
        .map(|u| u.name)
        .ok_or_else(|| {
            PrivilegeError::Probe(src.clone(), "cannot resolve the current user".into())
        })?;
    Ok(GrantPlan::Relocate {
        dest_dir: PathBuf::from(crate::tun::RELOCATE_DIR),
        dst: crate::tun::relocated_helper_path(),
        src,
        user,
    })
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
    format!("sudo setcap '{caps}' '{}'", path.display())
}

/// Builds the `pkexec` argument vector. Every binary path is passed as a
/// positional parameter (`$1`..`$9`) so the shell never parses it as code — a
/// user-controlled path containing shell metacharacters cannot inject commands.
///
/// `$9` is the last single-digit positional in POSIX `sh`; `$10` would be read
/// as `$1` followed by a literal `0`, so no plan may exceed nine.
fn grant_argv(backend: &Path, plan: &GrantPlan) -> Vec<OsString> {
    let mut argv = vec![
        OsString::from("/bin/sh"),
        OsString::from("-c"),
        OsString::from(grant_script(plan)),
        OsString::from("sh"),
        OsString::from(BACKEND_CAPS),
        backend.as_os_str().to_os_string(),
    ];

    match plan {
        GrantPlan::InPlace { helper, wrapper } => {
            argv.push(OsString::from(HELPER_CAPS));
            argv.push(helper.as_os_str().to_os_string());
            if let Some(wp) = wrapper {
                argv.push(wp.as_os_str().to_os_string());
            }
        }
        GrantPlan::Relocate {
            dest_dir,
            dst,
            user,
            src: _,
        } => {
            argv.push(dest_dir.as_os_str().to_os_string());
            argv.push(OsString::from(HELPER_GROUP));
            argv.push(dst.as_os_str().to_os_string());
            argv.push(OsString::from(HELPER_CAPS));
            argv.push(OsString::from(user));
        }
    }

    debug_assert!(
        argv.len() <= 4 + 9,
        "more positionals than POSIX sh can address"
    );
    argv
}

fn grant_script(plan: &GrantPlan) -> &'static str {
    match plan {
        GrantPlan::InPlace {
            wrapper: Some(_), ..
        } => {
            "setcap \"$1\" \"$2\" && setcap \"$3\" \"$4\" && chown root:root \"$5\" && chmod u+s \"$5\""
        }
        GrantPlan::InPlace { wrapper: None, .. } => "setcap \"$1\" \"$2\" && setcap \"$3\" \"$4\"",
        // The payload arrives on stdin, from a descriptor the unprivileged
        // caller opened: root cannot read an AppImage's FUSE mount at all.
        //
        // `chown` precedes `chmod`/`setcap` because it clears both file
        // capabilities and the setuid bit. The mode is set absolutely, and the
        // redirect creates a fresh inode, so no attribute of any previous file
        // at that path survives. The group is a dedicated system group rather
        // than the caller's primary group, which on distributions that share
        // one (gid 100 `users`) would hand every local account a
        // cap_net_admin binary.
        GrantPlan::Relocate { .. } => {
            "set -e\n\
             setcap \"$1\" \"$2\"\n\
             [ ! -L \"$3\" ]\n\
             mkdir -p \"$3\"\n\
             chown root:root \"$3\"\n\
             chmod 0755 \"$3\"\n\
             getent group \"$4\" >/dev/null 2>&1 || groupadd --system \"$4\"\n\
             rm -f \"$5\"\n\
             (umask 077; cat > \"$5\")\n\
             chown \"root:$4\" \"$5\"\n\
             chmod 0750 \"$5\"\n\
             setcap \"$6\" \"$5\"\n\
             id -nG \"$7\" | tr ' ' '\\n' | grep -qx \"$4\" || usermod -aG \"$4\" \"$7\"\n"
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
fn preflight_targets<'a>(backend: &'a Path, plan: &'a GrantPlan) -> Vec<(&'a Path, &'static str)> {
    let mut targets = vec![(backend, BACKEND_CAPS)];
    match plan {
        GrantPlan::InPlace { helper, wrapper } => {
            targets.push((helper.as_path(), HELPER_CAPS));
            if let Some(wp) = wrapper {
                targets.push((wp.as_path(), HELPER_CAPS));
            }
        }
        GrantPlan::Relocate { dest_dir, .. } => {
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
fn mount_for_path(mounts: &str, path: &Path) -> Option<String> {
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

    fn in_place(helper: &str, wrapper: Option<&str>) -> GrantPlan {
        GrantPlan::InPlace {
            helper: PathBuf::from(helper),
            wrapper: wrapper.map(PathBuf::from),
        }
    }

    fn relocate() -> GrantPlan {
        GrantPlan::Relocate {
            dest_dir: PathBuf::from("/usr/local/lib/v2ray-rs"),
            src: PathBuf::from("/tmp/.mount_abc/usr/bin/v2ray-rs-netctl"),
            dst: PathBuf::from("/usr/local/lib/v2ray-rs/v2ray-rs-netctl"),
            user: "zhuk".into(),
        }
    }

    fn strs(argv: &[OsString]) -> Vec<&str> {
        argv.iter().map(|a| a.to_str().unwrap()).collect()
    }

    #[test]
    fn grant_argv_runs_both_setcaps_in_one_elevation() {
        let argv = grant_argv(
            Path::new("/usr/bin/xray"),
            &in_place("/usr/bin/v2ray-rs-netctl", None),
        );
        assert_eq!(
            strs(&argv),
            vec![
                "/bin/sh",
                "-c",
                "setcap \"$1\" \"$2\" && setcap \"$3\" \"$4\"",
                "sh",
                BACKEND_CAPS,
                "/usr/bin/xray",
                HELPER_CAPS,
                "/usr/bin/v2ray-rs-netctl",
            ]
        );
    }

    #[test]
    fn grant_argv_neutralizes_shell_metacharacters_in_path() {
        // A malicious backend path must be passed verbatim as a positional
        // argument, never interpolated into the executed script.
        let argv = grant_argv(
            Path::new("/tmp/x'; touch /pwned; '"),
            &in_place("/usr/bin/helper", None),
        );
        assert_eq!(
            argv[2].to_str().unwrap(),
            "setcap \"$1\" \"$2\" && setcap \"$3\" \"$4\""
        );
        assert_eq!(argv[5].to_str().unwrap(), "/tmp/x'; touch /pwned; '");
    }

    #[test]
    fn grant_argv_includes_wrapper_step_when_present() {
        let argv = grant_argv(
            Path::new("/usr/bin/xray"),
            &in_place("/usr/bin/v2ray-rs-netctl", Some("/usr/bin/v2ray-rs-run")),
        );
        let argv = strs(&argv);
        assert_eq!(
            argv[2],
            "setcap \"$1\" \"$2\" && setcap \"$3\" \"$4\" && chown root:root \"$5\" && chmod u+s \"$5\""
        );
        // The wrapper survives verbatim as the trailing positional argument ($5).
        assert_eq!(argv[argv.len() - 1], "/usr/bin/v2ray-rs-run");
    }

    #[test]
    fn grant_argv_omits_wrapper_step_when_absent() {
        let argv = grant_argv(
            Path::new("/usr/bin/xray"),
            &in_place("/usr/bin/v2ray-rs-netctl", None),
        );
        assert_eq!(
            argv[2].to_str().unwrap(),
            "setcap \"$1\" \"$2\" && setcap \"$3\" \"$4\""
        );
        assert_eq!(argv.len(), 8);
    }

    #[test]
    fn grant_argv_keeps_metachar_wrapper_positional() {
        let argv = grant_argv(
            Path::new("/usr/bin/xray"),
            &in_place(
                "/usr/bin/v2ray-rs-netctl",
                Some("/tmp/run; chmod 777 /etc; #"),
            ),
        );
        // The script literal contains no interpolation of the path.
        assert_eq!(
            argv[2].to_str().unwrap(),
            "setcap \"$1\" \"$2\" && setcap \"$3\" \"$4\" && chown root:root \"$5\" && chmod u+s \"$5\""
        );
        // The dangerous path survives verbatim as the final positional argument.
        assert_eq!(
            argv[argv.len() - 1].to_str().unwrap(),
            "/tmp/run; chmod 777 /etc; #"
        );
    }

    #[test]
    fn relocating_grant_copies_then_grants_the_destination() {
        let argv = grant_argv(Path::new("/usr/bin/xray"), &relocate());
        assert_eq!(
            strs(&argv),
            vec![
                "/bin/sh",
                "-c",
                grant_script(&relocate()),
                "sh",
                BACKEND_CAPS,
                "/usr/bin/xray",
                "/usr/local/lib/v2ray-rs",
                HELPER_GROUP,
                "/usr/local/lib/v2ray-rs/v2ray-rs-netctl",
                HELPER_CAPS,
                "zhuk",
            ]
        );
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
        assert!(script.contains("usermod -aG \"$4\" \"$7\""));
        let argv = grant_argv(Path::new("/usr/bin/xray"), &relocate());
        assert_eq!(argv[7].to_str().unwrap(), "v2ray-rs");
    }

    #[test]
    fn relocating_grant_keeps_metachar_paths_positional() {
        let plan = GrantPlan::Relocate {
            dest_dir: PathBuf::from("/usr/local/lib/v2ray-rs"),
            src: PathBuf::from("/tmp/.mount_abc/usr/bin/v2ray-rs-netctl"),
            dst: PathBuf::from("/tmp/x\"; touch /pwned; \""),
            user: "zhuk".into(),
        };
        let argv = grant_argv(Path::new("/usr/bin/xray"), &plan);
        assert_eq!(argv[2].to_str().unwrap(), grant_script(&plan));
        assert_eq!(argv[8].to_str().unwrap(), "/tmp/x\"; touch /pwned; \"");
        // The source is the stdin payload now; it never reaches the shell.
        assert!(
            !argv
                .iter()
                .any(|a| a.to_str() == Some("/tmp/.mount_abc/usr/bin/v2ray-rs-netctl"))
        );
    }

    #[test]
    fn no_plan_exceeds_the_single_digit_positionals() {
        for plan in [
            in_place("/usr/bin/v2ray-rs-netctl", None),
            in_place("/usr/bin/v2ray-rs-netctl", Some("/usr/bin/v2ray-rs-run")),
            relocate(),
        ] {
            let argv = grant_argv(Path::new("/usr/bin/xray"), &plan);
            assert!(argv.len() <= 4 + 9, "{:?} needs ${}", plan, argv.len() - 4);
        }
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
    fn preflight_targets_includes_helper_and_optional_wrapper() {
        let backend = Path::new("/usr/bin/xray");
        let helper = Path::new("/usr/bin/v2ray-rs-netctl");
        let wrapper = Path::new("/usr/lib/v2ray-rs/v2ray-rs-run");

        let plan = in_place("/usr/bin/v2ray-rs-netctl", None);
        let without_wrapper = preflight_targets(backend, &plan);
        assert_eq!(without_wrapper.len(), 2);
        assert_eq!(without_wrapper[0].0, backend);
        assert_eq!(without_wrapper[0].1, BACKEND_CAPS);
        assert_eq!(without_wrapper[1].0, helper);
        assert_eq!(without_wrapper[1].1, HELPER_CAPS);

        let plan = in_place(
            "/usr/bin/v2ray-rs-netctl",
            Some("/usr/lib/v2ray-rs/v2ray-rs-run"),
        );
        let with_wrapper = preflight_targets(backend, &plan);
        assert_eq!(with_wrapper.len(), 3);
        assert_eq!(with_wrapper[2].0, wrapper);
        assert_eq!(with_wrapper[2].1, HELPER_CAPS);
    }

    #[test]
    fn relocating_preflight_checks_the_destination_not_the_appdir() {
        let plan = relocate();
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
}
