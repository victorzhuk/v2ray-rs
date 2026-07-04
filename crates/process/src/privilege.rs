use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// File capabilities the backend binary needs for TUN: create the device
/// (`cap_net_admin`), bind privileged ports (`cap_net_bind_service`), and open
/// raw sockets (`cap_net_raw`).
pub const BACKEND_CAPS: &str = "cap_net_admin,cap_net_bind_service,cap_net_raw+ep";

/// File capability the route helper needs: program addresses and routes.
pub const HELPER_CAPS: &str = "cap_net_admin+ep";

#[derive(Debug, thiserror::Error)]
pub enum PrivilegeError {
    #[error("read capabilities of {0}: {1}")]
    Probe(PathBuf, String),
    #[error(
        "{path} is on a filesystem that ignores file capabilities (e.g. mounted nosuid). \
         Grant manually after moving the binary, or run: sudo setcap '{caps}' {path}"
    )]
    Unsupported { path: PathBuf, caps: String },
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

/// Grants the required capabilities to both binaries via a single `pkexec`
/// elevation. Blocks until the polkit dialog completes.
pub fn grant(backend: &Path, helper: &Path) -> Result<(), PrivilegeError> {
    if !file_caps_supported(backend) {
        return Err(PrivilegeError::Unsupported {
            path: backend.to_path_buf(),
            caps: BACKEND_CAPS.to_string(),
        });
    }

    // Resolve helper and wrapper to absolute sibling paths before handing them
    // to a root-elevated setcap/chown/chmod. A bare or relative path would be
    // resolved against the elevated process's CWD, so a planted file could be
    // given caps or the setuid bit; refuse anything not found beside our exe.
    let helper = if helper.is_absolute() {
        helper.to_path_buf()
    } else {
        crate::tun::helper_path_strict().ok_or_else(|| {
            PrivilegeError::Probe(
                helper.to_path_buf(),
                "route helper not found beside the executable".into(),
            )
        })?
    };
    let wrapper = crate::tun::run_path_strict();
    let argv = grant_argv(backend, &helper, wrapper.as_deref());
    let status = Command::new("pkexec")
        .args(&argv)
        .status()
        .map_err(|e| PrivilegeError::Spawn("pkexec", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(PrivilegeError::GrantFailed)
    }
}

/// The manual `setcap` command shown to the user when an automatic grant is not
/// possible.
pub fn manual_command(path: &Path, caps: &str) -> String {
    format!("sudo setcap '{caps}' '{}'", path.display())
}

/// Builds the `pkexec` argument vector: a single elevation running both
/// `setcap` calls and, when a SUID wrapper is present, additionally chowning
/// it to root and setting the setuid bit. All binary paths are passed as
/// positional parameters (`$1`..`$5`) so the shell never parses them as code
/// — a user-controlled path containing shell metacharacters cannot inject
/// commands.
fn grant_argv(backend: &Path, helper: &Path, wrapper: Option<&Path>) -> Vec<OsString> {
    let script = match wrapper {
        Some(_) => {
            "setcap \"$1\" \"$2\" && setcap \"$3\" \"$4\" && chown root:root \"$5\" && chmod u+s \"$5\""
        }
        None => "setcap \"$1\" \"$2\" && setcap \"$3\" \"$4\"",
    };
    let mut argv = vec![
        OsString::from("/bin/sh"),
        OsString::from("-c"),
        OsString::from(script),
        OsString::from("sh"),
        OsString::from(BACKEND_CAPS),
        backend.as_os_str().to_os_string(),
        OsString::from(HELPER_CAPS),
        helper.as_os_str().to_os_string(),
    ];
    if let Some(wp) = wrapper {
        argv.push(wp.as_os_str().to_os_string());
    }
    argv
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

/// Best-effort check that the filesystem holding `path` honors file
/// capabilities. A `nosuid` mount silently ignores them. Defaults to `true`
/// when the mount cannot be determined.
fn file_caps_supported(path: &Path) -> bool {
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
        let covers = path == mount_point
            || (path.starts_with(mount_point)
                && (mount_point == "/" || path.as_bytes().get(mount_point.len()) == Some(&b'/')));
        if covers
            && best
                .as_ref()
                .is_none_or(|(len, _)| mount_point.len() > *len)
        {
            best = Some((mount_point.len(), opts.to_string()));
        }
    }
    best.map(|(_, opts)| opts)
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

    #[test]
    fn grant_argv_runs_both_setcaps_in_one_elevation() {
        let argv = grant_argv(
            Path::new("/usr/bin/xray"),
            Path::new("/usr/bin/v2ray-rs-netctl"),
            None,
        );
        let argv: Vec<&str> = argv.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            argv,
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
            Path::new("/usr/bin/helper"),
            None,
        );
        assert_eq!(
            argv[2].to_str().unwrap(),
            "setcap \"$1\" \"$2\" && setcap \"$3\" \"$4\""
        );
        assert_eq!(argv[5].to_str().unwrap(), "/tmp/x'; touch /pwned; '");
    }

    #[test]
    fn grant_argv_includes_wrapper_step_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let wrapper = dir.path().join("v2ray-rs-run");
        std::fs::write(&wrapper, b"#!/bin/sh\n").unwrap();
        let argv = grant_argv(
            Path::new("/usr/bin/xray"),
            Path::new("/usr/bin/v2ray-rs-netctl"),
            Some(&wrapper),
        );
        let argv: Vec<&str> = argv.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            argv[2],
            "setcap \"$1\" \"$2\" && setcap \"$3\" \"$4\" && chown root:root \"$5\" && chmod u+s \"$5\""
        );
        // The wrapper survives verbatim as the trailing positional argument ($5).
        assert_eq!(argv[argv.len() - 1], wrapper.to_str().unwrap());
    }

    #[test]
    fn grant_argv_omits_wrapper_step_when_absent() {
        let argv = grant_argv(
            Path::new("/usr/bin/xray"),
            Path::new("/usr/bin/v2ray-rs-netctl"),
            None,
        );
        let argv: Vec<&str> = argv.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(argv[2], "setcap \"$1\" \"$2\" && setcap \"$3\" \"$4\"");
        assert_eq!(argv.len(), 8);
    }

    #[test]
    fn grant_argv_keeps_metachar_wrapper_positional() {
        let wrapper = Path::new("/tmp/run; chmod 777 /etc; #");
        let argv = grant_argv(
            Path::new("/usr/bin/xray"),
            Path::new("/usr/bin/v2ray-rs-netctl"),
            Some(wrapper),
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
}
