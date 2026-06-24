use std::ffi::CString;
use std::os::unix::ffi::OsStringExt;

use libc::{c_char, gid_t, uid_t};

const BYPASS_USER: &str = "v2ray-rs-bypass";

const DANGEROUS_ENV: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "LD_DEBUG",
    "LD_ORIGIN_PATH",
    "GCONV_PATH",
    "HOSTALIASES",
    "IFS",
    "TMPDIR",
    "NLSPATH",
];

const SAFE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

fn main() {
    let argv = match read_argv() {
        Ok(v) => v,
        Err(e) => die(e),
    };
    let command = match command_argv(&argv) {
        Ok(c) => c,
        Err(e) => die(e),
    };
    let (uid, gid) = match resolve_bypass_user() {
        Ok(v) => v,
        Err(e) => die(e),
    };
    if let Err(e) = drop_privileges(uid, gid) {
        die(e);
    }
    let sanitized = sanitize_env(&std::env::vars().collect::<Vec<_>>());
    apply_env(&sanitized);
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

fn resolve_bypass_user() -> Result<(uid_t, gid_t), &'static str> {
    resolve_user(BYPASS_USER)
}

fn resolve_user(name: &str) -> Result<(uid_t, gid_t), &'static str> {
    let name = CString::new(name).map_err(|_| "invalid user name")?;
    let (uid, gid) = unsafe {
        let pw = libc::getpwnam(name.as_ptr());
        if pw.is_null() {
            return Err("user not found");
        }
        ((*pw).pw_uid, (*pw).pw_gid)
    };
    if uid == 0 || gid == 0 {
        return Err("bypass user must not resolve to root");
    }
    Ok((uid, gid))
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
        if libc::getuid() != uid || libc::geteuid() != uid || libc::getgid() != gid {
            return Err("privilege drop verification failed");
        }
    }
    Ok(())
}

fn sanitize_env(input: &[(String, String)]) -> Vec<(String, String)> {
    input
        .iter()
        .filter(|(k, _)| !DANGEROUS_ENV.contains(&k.as_str()) && k != "PATH")
        .cloned()
        .chain([("PATH".to_owned(), SAFE_PATH.to_owned())])
        .collect()
}

fn apply_env(sanitized: &[(String, String)]) {
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
    fn sanitize_env_removes_dangerous_and_forces_path() {
        let input: Vec<(String, String)> = DANGEROUS_ENV
            .iter()
            .map(|k| ((*k).to_owned(), "/evil".to_owned()))
            .chain([
                ("HOME".to_owned(), "/home/u".to_owned()),
                ("USER".to_owned(), "u".to_owned()),
                ("PATH".to_owned(), "/attacker/old".to_owned()),
            ])
            .collect();

        let out = sanitize_env(&input);
        let keys: Vec<&str> = out.iter().map(|(k, _)| k.as_str()).collect();

        for danger in DANGEROUS_ENV {
            assert!(!keys.contains(danger), "{danger} should be stripped");
        }
        assert!(keys.contains(&"HOME"), "HOME should be preserved");
        assert!(keys.contains(&"USER"), "USER should be preserved");

        let paths: Vec<&String> = out
            .iter()
            .filter(|(k, _)| k == "PATH")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(paths.len(), 1, "exactly one PATH entry");
        assert_eq!(paths[0], SAFE_PATH);
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
}
