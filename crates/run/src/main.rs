use std::ffi::{CStr, CString, OsString};
#[cfg(test)]
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;

use libc::{c_char, gid_t, uid_t};

const BYPASS_USER: &str = "v2ray-rs-bypass";

/// Only session variables a desktop application needs. Everything else is
/// dropped, including loader, interpreter, and module-injection hooks.
const ALLOWED_ENV: &[&str] = &[
    "TERM",
    "LANG",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "XAUTHORITY",
];

const SAFE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

struct BypassUser {
    uid: uid_t,
    gid: gid_t,
    home: Option<String>,
}

fn main() {
    let argv = match read_argv() {
        Ok(v) => v,
        Err(e) => die(e),
    };
    let command = match command_argv(&argv) {
        Ok(c) => c,
        Err(e) => die(e),
    };
    let user = match resolve_bypass_user() {
        Ok(v) => v,
        Err(e) => die(e),
    };
    if let Err(e) = drop_privileges(user.uid, user.gid) {
        die(e);
    }
    let sanitized = sanitize_env(&std::env::vars_os().collect::<Vec<_>>(), &user);
    apply_env(&sanitized);
    close_inherited_fds();
    exec_command(&command);
}

fn read_argv() -> Result<Vec<CString>, &'static str> {
    std::env::args_os()
        .map(|s| CString::new(s.into_vec()).map_err(|_| "argv contains invalid byte"))
        .collect()
}

fn command_argv(argv: &[CString]) -> Result<Vec<CString>, &'static str> {
    if argv.len() < 2 {
        return Err("usage: v2ray-rs-run <command> [args...]");
    }
    Ok(argv[1..].to_vec())
}

fn resolve_bypass_user() -> Result<BypassUser, &'static str> {
    resolve_user(BYPASS_USER)
}

fn resolve_user(name: &str) -> Result<BypassUser, &'static str> {
    let name = CString::new(name).map_err(|_| "invalid user name")?;
    let (uid, gid, home) = unsafe {
        let pw = libc::getpwnam(name.as_ptr());
        if pw.is_null() {
            return Err("user not found");
        }
        let home = if (*pw).pw_dir.is_null() {
            String::new()
        } else {
            CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned()
        };
        ((*pw).pw_uid, (*pw).pw_gid, home)
    };
    if uid == 0 || gid == 0 {
        return Err("bypass user must not resolve to root");
    }
    Ok(BypassUser {
        uid,
        gid,
        home: (!home.is_empty()).then_some(home),
    })
}

fn drop_privileges(uid: uid_t, gid: gid_t) -> Result<(), &'static str> {
    unsafe {
        if libc::setgroups(0, std::ptr::null()) != 0 {
            return Err("setgroups failed");
        }
        if libc::setresgid(gid, gid, gid) != 0 {
            return Err("setresgid failed");
        }
        if libc::setresuid(uid, uid, uid) != 0 {
            return Err("setresuid failed");
        }

        let (mut ruid, mut euid, mut suid) = (0, 0, 0);
        if libc::getresuid(&mut ruid, &mut euid, &mut suid) != 0 {
            return Err("getresuid failed");
        }
        if ruid != uid || euid != uid || suid != uid {
            return Err("uid drop verification failed");
        }

        let (mut rgid, mut egid, mut sgid) = (0, 0, 0);
        if libc::getresgid(&mut rgid, &mut egid, &mut sgid) != 0 {
            return Err("getresgid failed");
        }
        if rgid != gid || egid != gid || sgid != gid {
            return Err("gid drop verification failed");
        }

        if libc::getgroups(0, std::ptr::null_mut()) != 0 {
            return Err("supplementary groups were not cleared");
        }
    }
    Ok(())
}

fn sanitize_env(input: &[(OsString, OsString)], user: &BypassUser) -> Vec<(OsString, OsString)> {
    let mut out: Vec<(OsString, OsString)> = input
        .iter()
        .filter(|(k, _)| {
            // Only a UTF-8 key can match the allowlist; any other key is
            // dropped and its value bytes are never inspected.
            let name = k.to_str();
            k != "PATH"
                && k != "HOME"
                && k != "USER"
                && k != "LOGNAME"
                && name.is_some_and(|name| ALLOWED_ENV.contains(&name) || name.starts_with("LC_"))
        })
        .cloned()
        .collect();
    out.push((OsString::from("PATH"), OsString::from(SAFE_PATH)));
    if let Some(home) = &user.home {
        out.push((OsString::from("HOME"), OsString::from(home)));
    }
    out.push((OsString::from("USER"), OsString::from(BYPASS_USER)));
    out.push((OsString::from("LOGNAME"), OsString::from(BYPASS_USER)));
    out
}

fn apply_env(sanitized: &[(OsString, OsString)]) {
    let keys: Vec<std::ffi::OsString> = std::env::vars_os().map(|(k, _)| k).collect();
    unsafe {
        for k in keys {
            std::env::remove_var(k);
        }
        for (k, v) in sanitized {
            std::env::set_var(k, v);
        }
    }
}

fn close_inherited_fds() {
    let rc = unsafe {
        libc::syscall(
            libc::SYS_close_range,
            3 as libc::c_uint,
            libc::c_uint::MAX,
            0 as libc::c_uint,
        )
    };
    if rc == 0 {
        return;
    }
    close_fds_via_procfs();
}

fn close_fds_via_procfs() {
    unsafe {
        let dir = libc::opendir(c"/proc/self/fd".as_ptr());
        if dir.is_null() {
            return;
        }
        let dir_fd = libc::dirfd(dir);
        let mut to_close = Vec::new();
        loop {
            let entry = libc::readdir(dir);
            if entry.is_null() {
                break;
            }
            let name = CStr::from_ptr((*entry).d_name.as_ptr());
            if let Ok(text) = name.to_str()
                && let Ok(fd) = text.parse::<libc::c_int>()
                && fd >= 3
                && fd != dir_fd
            {
                to_close.push(fd);
            }
        }
        libc::closedir(dir);
        for fd in to_close {
            libc::close(fd);
        }
    }
}

fn exec_command(command: &[CString]) -> ! {
    let mut argv: Vec<*const c_char> = command.iter().map(|s| s.as_ptr()).collect();
    argv.push(std::ptr::null());
    unsafe {
        libc::execvp(argv[0], argv.as_ptr());
    }
    die("execvp failed");
}

fn die(msg: &str) -> ! {
    eprintln!("v2ray-rs-run: {msg}");
    unsafe {
        libc::_exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn cstrs(items: &[&str]) -> Vec<CString> {
        items.iter().map(|s| CString::new(*s).unwrap()).collect()
    }

    fn names(vec: &[CString]) -> Vec<&str> {
        vec.iter().map(|c| c.as_c_str().to_str().unwrap()).collect()
    }

    #[test]
    fn command_argv_drops_program_name_and_preserves_order() {
        let argv = cstrs(&["v2ray-rs-run", "curl", "ifconfig.me", "--silent"]);
        let command = command_argv(&argv).unwrap();
        assert_eq!(names(&command), &["curl", "ifconfig.me", "--silent"]);
    }

    #[test]
    fn command_argv_rejects_missing_command() {
        let argv = cstrs(&["v2ray-rs-run"]);
        assert!(command_argv(&argv).is_err());
    }

    #[test]
    fn command_argv_rejects_empty() {
        assert!(command_argv(&[]).is_err());
    }

    #[test]
    fn sanitize_env_keeps_only_session_variables() {
        let user = BypassUser {
            uid: 12345,
            gid: 12345,
            home: Some("/nonexistent".to_owned()),
        };
        let input: Vec<(OsString, OsString)> = [
            ("PYTHONPATH", "/evil"),
            ("LD_AUDIT", "/evil.so"),
            ("LD_PRELOAD", "/evil.so"),
            ("GIO_EXTRA_MODULES", "/evil"),
            ("GTK_MODULES", "evil"),
            ("GCONV_PATH", "/evil"),
            ("BASH_ENV", "/evil.sh"),
            ("HOSTALIASES", "/evil"),
            ("PATH", "/attacker/old"),
            ("HOME", "/home/attacker"),
            ("USER", "attacker"),
            ("LOGNAME", "attacker"),
            ("DISPLAY", ":0"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("XAUTHORITY", "/run/user/1000/.Xauthority"),
            ("LANG", "en_US.UTF-8"),
            ("LC_ALL", "C.UTF-8"),
            ("TERM", "xterm-256color"),
        ]
        .into_iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        .collect();

        let out = sanitize_env(&input, &user);
        let keys: Vec<&str> = out.iter().map(|(k, _)| k.to_str().unwrap()).collect();

        for stripped in [
            "PYTHONPATH",
            "LD_AUDIT",
            "LD_PRELOAD",
            "GIO_EXTRA_MODULES",
            "GTK_MODULES",
            "GCONV_PATH",
            "BASH_ENV",
            "HOSTALIASES",
        ] {
            assert!(!keys.contains(&stripped), "{stripped} should be stripped");
        }
        for kept in [
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XDG_RUNTIME_DIR",
            "DBUS_SESSION_BUS_ADDRESS",
            "XAUTHORITY",
            "LANG",
            "LC_ALL",
            "TERM",
        ] {
            assert!(keys.contains(&kept), "{kept} should be preserved");
        }

        let value = |key: &str| {
            out.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.to_str().unwrap())
                .unwrap()
        };
        assert_eq!(value("PATH"), SAFE_PATH);
        assert_eq!(value("HOME"), "/nonexistent");
        assert_eq!(value("USER"), BYPASS_USER);
        assert_eq!(value("LOGNAME"), BYPASS_USER);
        assert_eq!(
            out.iter().filter(|(k, _)| k == "PATH").count(),
            1,
            "exactly one PATH entry"
        );
    }

    #[test]
    fn sanitize_env_drops_non_utf8_keys_without_inspecting_values() {
        let user = BypassUser {
            uid: 12345,
            gid: 12345,
            home: None,
        };
        let input = vec![
            (
                OsString::from_vec(vec![b'B', 0xff]),
                OsString::from_vec(vec![0x80]),
            ),
            (
                OsString::from("LC_CTYPE"),
                OsString::from_vec(vec![b'x', 0xfe]),
            ),
        ];

        let out = sanitize_env(&input, &user);

        for (k, _) in &out {
            assert!(
                !k.as_bytes().contains(&0xff),
                "non-UTF-8 key must be dropped"
            );
        }
        let lc = out
            .iter()
            .find(|(k, _)| k.as_bytes() == b"LC_CTYPE")
            .expect("LC_CTYPE must be preserved");
        assert_eq!(lc.1.as_bytes(), b"x\xfe");
    }

    #[test]
    fn resolve_user_rejects_root() {
        assert!(resolve_user("root").is_err());
    }

    #[test]
    fn resolve_user_rejects_missing_user() {
        assert!(resolve_user("v2ray-rs-bypass-definitely-missing").is_err());
    }

    #[test]
    fn drop_privileges_fails_when_not_root() {
        // When running as an unprivileged user, setgroups(0) returns EPERM,
        // so the drop aborts before any exec path can be reached.
        if unsafe { libc::getuid() } == 0 {
            return;
        }
        assert!(drop_privileges(12345, 12345).is_err());
    }

    #[test]
    fn read_argv_rejects_interior_nul() {
        // Simulate an argv entry containing an interior NUL byte.
        let bad = std::ffi::OsString::from_vec(vec![0x2f, 0x00, 0x2f]);
        let result: Result<Vec<CString>, &'static str> = std::iter::once(bad)
            .map(|s| CString::new(s.into_vec()).map_err(|_| "argv contains invalid byte"))
            .collect();
        assert!(result.is_err());
    }

    #[test]
    fn close_inherited_fds_closes_non_cloexec_descriptors() {
        let dev_null = CString::new("/dev/null").unwrap();
        let fd = unsafe { libc::open(dev_null.as_ptr(), libc::O_RDONLY) };
        assert!(fd >= 3, "expected fd >= 3, got {fd}");
        unsafe {
            assert_eq!(libc::fcntl(fd, libc::F_SETFD, 0), 0);
        }

        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            close_inherited_fds();
            let still_open = unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1;
            unsafe { libc::_exit(if still_open { 2 } else { 0 }) };
        }

        let mut status = 0;
        let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
        unsafe { libc::close(fd) };
        assert_eq!(waited, pid);
        assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0);
    }

    #[test]
    fn close_fds_via_procfs_closes_non_cloexec_descriptors() {
        let dev_null = CString::new("/dev/null").unwrap();
        let fd = unsafe { libc::open(dev_null.as_ptr(), libc::O_RDONLY) };
        assert!(fd >= 3, "expected fd >= 3, got {fd}");
        unsafe {
            assert_eq!(libc::fcntl(fd, libc::F_SETFD, 0), 0);
        }

        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            close_fds_via_procfs();
            let still_open = unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1;
            unsafe { libc::_exit(if still_open { 2 } else { 0 }) };
        }

        let mut status = 0;
        let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
        unsafe { libc::close(fd) };
        assert_eq!(waited, pid);
        assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0);
    }
}
