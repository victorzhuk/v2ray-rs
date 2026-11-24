use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(test)]
use nix::errno::Errno;
#[cfg(test)]
use nix::sys::signal::kill;
#[cfg(test)]
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};

fn canonicalize_best(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
fn process_exists(pid: u32) -> bool {
    matches!(
        kill(Pid::from_raw(pid as i32), None),
        Ok(()) | Err(Errno::EPERM)
    )
}

/// Time allowed for the pinned process to exit after each signal.
const TERM_EXIT_TIMEOUT: Duration = Duration::from_secs(1);
const KILL_EXIT_TIMEOUT: Duration = Duration::from_millis(500);

/// Why a `pidfd_open(2)` call did not return a pinning descriptor.
#[derive(Debug)]
enum OpenFailure {
    /// `ESRCH`: the process already exited; nothing is left to clean up.
    Gone,
    /// `ENOSYS`: the kernel predates pidfd support. Cleanup cannot be
    /// performed safely, so the caller must fail closed.
    Unsupported,
    /// Any other failure, carrying the OS error.
    Failed(io::Error),
}

fn open_disposition(raw_errno: Option<i32>) -> OpenFailure {
    match raw_errno {
        Some(nix::libc::ESRCH) => OpenFailure::Gone,
        Some(nix::libc::ENOSYS) => OpenFailure::Unsupported,
        _ => OpenFailure::Failed(io::Error::last_os_error()),
    }
}

/// Outcome for the ownership record once termination of the pinned process
/// has been attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordDisposition {
    /// Exit is confirmed; the stale record must be dropped and an orphan
    /// reported.
    DropConfirmedExit,
    /// Exit did not confirm inside the budget; the record must stay so a
    /// replacement startup remains blocked.
    KeepUnconfirmed,
}

/// Pure decision point for the TERM-then-KILL escalation ladder. `kill_exit`
/// is `None` while the SIGKILL phase has not started; `Some(true)` only after
/// the pinned process's exit was confirmed in the kill phase.
fn record_disposition(term_exit_confirmed: bool, kill_exit: Option<bool>) -> RecordDisposition {
    if term_exit_confirmed || kill_exit == Some(true) {
        RecordDisposition::DropConfirmedExit
    } else {
        RecordDisposition::KeepUnconfirmed
    }
}

/// A pidfd pinning one process identity. Signals sent through it can never
/// reach a different process that reused the PID, and the kernel reports the
/// pinned process's termination through this descriptor.
pub(crate) struct PinnedProcess {
    fd: OwnedFd,
}

impl PinnedProcess {
    /// Safety: `SYS_pidfd_open(pid, 0)` returns a pidfd owned by the caller
    /// on success and -1 with errno on failure; the descriptor is handed to
    /// `OwnedFd`, which closes it on drop.
    fn open(pid: u32) -> Result<Self, OpenFailure> {
        let rc = unsafe {
            nix::libc::syscall(
                nix::libc::SYS_pidfd_open,
                pid as nix::libc::pid_t,
                0 as nix::libc::c_uint,
            )
        };
        if rc >= 0 {
            // Safety: rc is a fresh, exclusively owned file descriptor.
            return Ok(Self {
                fd: unsafe { OwnedFd::from_raw_fd(rc as nix::libc::c_int) },
            });
        }
        Err(open_disposition(io::Error::last_os_error().raw_os_error()))
    }

    /// Pins a live-or-exited process identity for the engine monitor, or
    /// `None` when this kernel or context cannot pin one (ENOSYS, EPERM, a
    /// pid already reaped). Must be called while the `Child` is still owned,
    /// before any wait can reap the process.
    pub(crate) fn pin(pid: u32) -> Option<Self> {
        Self::open(pid).ok()
    }

    /// `pidfd_send_signal(2)`. `Ok(false)` maps `ESRCH`: the pinned process
    /// exited and was reaped before the signal landed.
    fn send_signal(&self, sig: nix::libc::c_int) -> io::Result<bool> {
        // Safety: the pidfd is a live descriptor owned by `self`; a null
        // `siginfo_t` and zero flags deliver the plain signal.
        let rc = unsafe {
            nix::libc::syscall(
                nix::libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                sig,
                std::ptr::null_mut::<nix::libc::siginfo_t>(),
                0 as nix::libc::c_uint,
            )
        };
        if rc >= 0 {
            return Ok(true);
        }
        if io::Error::last_os_error().raw_os_error() == Some(nix::libc::ESRCH) {
            Ok(false)
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Blocks until the kernel reports the pinned process terminated through
    /// the pidfd, or the deadline passes. `true` only for confirmed exit.
    fn wait_exit(&self, timeout: Duration) -> io::Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            let mut fds = nix::libc::pollfd {
                fd: self.fd.as_raw_fd(),
                events: nix::libc::POLLIN,
                revents: 0,
            };
            // Safety: `fds` points at one valid `pollfd` for the duration of
            // the call.
            let rc = unsafe {
                nix::libc::poll(
                    &mut fds,
                    1,
                    (deadline - now).as_millis() as nix::libc::c_int,
                )
            };
            if rc < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            if rc > 0 && fds.revents & nix::libc::POLLIN != 0 {
                return Ok(true);
            }
            if rc > 0 {
                return Err(io::Error::other("unexpected pidfd readiness"));
            }
        }
    }
}

enum ExeLookup {
    Path(PathBuf),
    PermissionDenied,
    Missing,
}

/// One nonblocking poll of the pidfd: `true` only when the kernel has
/// reported the pinned process terminated.
fn pid_exited(pinned: &PinnedProcess) -> bool {
    let mut fds = nix::libc::pollfd {
        fd: pinned.fd.as_raw_fd(),
        events: nix::libc::POLLIN,
        revents: 0,
    };
    // Safety: `fds` points at one valid `pollfd` for the duration of the call.
    let rc = unsafe { nix::libc::poll(&mut fds, 1, 0) };
    rc > 0 && fds.revents & nix::libc::POLLIN != 0
}

/// Bounded exit observation through a pinned process: an already-exited
/// process confirms at once; a live one gets TERM then, if still alive after
/// the TERM budget, KILL through the pidfd — never through a re-looked-up
/// numeric PID. `true` only for a confirmed exit; no exit code is produced.
/// Run on a blocking worker: `wait_exit` parks in `poll(2)`.
pub(crate) fn confirm_exit_by_pidfd(pinned: PinnedProcess) -> bool {
    if pid_exited(&pinned) {
        return true;
    }
    match pinned.send_signal(nix::libc::SIGTERM) {
        // ESRCH: exited and reaped between the poll and the signal.
        Ok(false) => return true,
        Ok(true) => {
            if pinned.wait_exit(TERM_EXIT_TIMEOUT).unwrap_or(false) {
                return true;
            }
        }
        Err(_) => {}
    }
    if matches!(pinned.send_signal(nix::libc::SIGKILL), Ok(false)) {
        return true;
    }
    pinned.wait_exit(KILL_EXIT_TIMEOUT).unwrap_or(false)
}

fn process_binary_path(pid: u32) -> ExeLookup {
    match fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(path) => ExeLookup::Path(path),
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => ExeLookup::PermissionDenied,
        Err(_) => ExeLookup::Missing,
    }
}

fn process_comm(pid: u32) -> Option<String> {
    let raw = fs::read(format!("/proc/{pid}/comm")).ok()?;
    Some(String::from_utf8_lossy(&raw).trim_end().to_string())
}

fn process_config_path(pid: u32) -> Option<PathBuf> {
    let raw = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args: Vec<String> = raw
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();

    let config_arg = args.windows(2).find_map(|window| match window {
        [flag, value] if flag == "-c" || flag == "--config" => Some(value),
        _ => None,
    })?;
    Some(PathBuf::from(config_arg))
}

pub(crate) fn process_start_time(pid: u32) -> io::Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let Some((_, rest)) = stat.rsplit_once(") ") else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected /proc stat format",
        ));
    };
    let Some(start_time) = rest.split_whitespace().nth(19) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing process start time",
        ));
    };
    start_time
        .parse::<u64>()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

/// `pid:start_time_ticks` for this process — the owner identity the manager
/// hands to `v2ray-rs-antidpi --owner`, in the helper's decimal `PID:START`
/// format. `None` when `/proc` is unreadable; callers then omit the flag and
/// the helper falls back to its parent process, which is this process anyway.
pub(crate) fn owner_identity() -> Option<String> {
    let pid = std::process::id();
    let start_time = process_start_time(pid).ok()?;
    Some(format!("{pid}:{start_time}"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidOwnershipRecord {
    pub pid: u32,
    pub binary_path: PathBuf,
    pub config_path: PathBuf,
    pub start_time_ticks: u64,
}

fn atomic_write_0600(path: &Path, payload: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        // A bare file name lives in the working directory.
        _ => Path::new("."),
    };
    // A unique temp file per write: two writers in one process cannot
    // create, overwrite, or remove each other's staging file.
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(payload)?;
    tmp.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|err| err.error)?;
    Ok(())
}

enum PidFileEntry {
    Record(PidOwnershipRecord),
    Legacy,
    Corrupt,
}

#[derive(Clone)]
pub struct PidFile {
    path: PathBuf,
}

impl PidFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn write(&self, pid: u32, binary_path: &Path, config_path: &Path) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let record = PidOwnershipRecord {
            pid,
            binary_path: canonicalize_best(binary_path),
            config_path: canonicalize_best(config_path),
            start_time_ticks: process_start_time(pid)?,
        };

        let payload = serde_json::to_vec_pretty(&record)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        atomic_write_0600(&self.path, &payload)?;

        Ok(())
    }

    pub fn read(&self) -> io::Result<Option<PidOwnershipRecord>> {
        match self.read_entry()? {
            None => Ok(None),
            Some(PidFileEntry::Record(record)) => Ok(Some(record)),
            Some(PidFileEntry::Legacy) | Some(PidFileEntry::Corrupt) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "pid file is not a structured ownership record",
            )),
        }
    }

    pub fn remove(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn check_and_kill_orphaned(&self) -> io::Result<bool> {
        let Some(entry) = self.read_entry()? else {
            return Ok(false);
        };

        let record = match entry {
            PidFileEntry::Record(record) => record,
            PidFileEntry::Legacy | PidFileEntry::Corrupt => {
                self.remove()?;
                return Ok(false);
            }
        };

        // Pin the process identity before verifying or signaling: the pidfd
        // keeps referring to this exact process even across PID reuse.
        let pinned = match PinnedProcess::open(record.pid) {
            Ok(pinned) => pinned,
            Err(OpenFailure::Gone) => {
                self.remove()?;
                return Ok(false);
            }
            Err(OpenFailure::Unsupported) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "pidfd_open unsupported: cannot safely clean up the recorded process",
                ));
            }
            Err(OpenFailure::Failed(err)) => return Err(err),
        };

        // Identity mismatch means the pid was already reused before pinning;
        // leave the replacement process alone and drop the stale record.
        if !process_matches_record(&record) {
            self.remove()?;
            return Ok(false);
        }

        match pinned.send_signal(nix::libc::SIGTERM) {
            Ok(true) => {}
            Ok(false) => {
                self.remove()?;
                return Ok(true);
            }
            Err(err) => return Err(err),
        }

        let disposition = match pinned.wait_exit(TERM_EXIT_TIMEOUT)? {
            true => record_disposition(true, None),
            false => {
                // Escalate through the pinned identity: a reused PID can
                // never receive this kill, so the start-time re-check is no
                // longer needed. The kill itself is not proof of exit: wait
                // for the pinned process to actually terminate.
                let kill_exit = match pinned.send_signal(nix::libc::SIGKILL)? {
                    // ESRCH: the pinned process exited before the kill landed.
                    false => true,
                    true => pinned.wait_exit(KILL_EXIT_TIMEOUT)?,
                };
                record_disposition(false, Some(kill_exit))
            }
        };

        match disposition {
            RecordDisposition::DropConfirmedExit => {
                self.remove()?;
                Ok(true)
            }
            RecordDisposition::KeepUnconfirmed => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "recorded process did not confirm exit; keeping the pid record",
            )),
        }
    }

    fn read_entry(&self) -> io::Result<Option<PidFileEntry>> {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };

        let trimmed = contents.trim();
        if trimmed.is_empty() {
            return Ok(Some(PidFileEntry::Corrupt));
        }

        if trimmed.parse::<u32>().is_ok() {
            return Ok(Some(PidFileEntry::Legacy));
        }

        match serde_json::from_str::<PidOwnershipRecord>(trimmed) {
            Ok(record) => Ok(Some(PidFileEntry::Record(record))),
            Err(_) => Ok(Some(PidFileEntry::Corrupt)),
        }
    }
}

fn process_matches_record(record: &PidOwnershipRecord) -> bool {
    let exe = process_binary_path(record.pid);
    let Some(actual_config_path) = process_config_path(record.pid) else {
        return false;
    };
    let Ok(actual_start_time) = process_start_time(record.pid) else {
        return false;
    };
    let comm = process_comm(record.pid);

    matches_record_parts(
        record,
        exe,
        actual_start_time,
        Some(&actual_config_path),
        comm.as_deref(),
    )
}

/// Verifies the recorded identity. `exe` is the `/proc/<pid>/exe` result:
/// capability-bearing binaries on a `fs.suid_dumpable = 2` host expose it only
/// to root, so the fallback trusts start ticks plus `comm` plus the exact
/// config argument instead.
fn matches_record_parts(
    record: &PidOwnershipRecord,
    exe: ExeLookup,
    actual_start_time: u64,
    actual_config_path: Option<&Path>,
    actual_comm: Option<&str>,
) -> bool {
    if actual_start_time != record.start_time_ticks {
        return false;
    }

    let Some(actual_config_path) = actual_config_path else {
        return false;
    };
    if canonicalize_best(actual_config_path) != record.config_path {
        return false;
    }

    match exe {
        ExeLookup::Path(path) => canonicalize_best(&path) == record.binary_path,
        ExeLookup::PermissionDenied => actual_comm.is_some_and(|comm| comm_matches(comm, record)),
        ExeLookup::Missing => false,
    }
}

fn comm_matches(comm: &str, record: &PidOwnershipRecord) -> bool {
    let Some(name) = record.binary_path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let expected = &name.as_bytes()[..name.len().min(15)];
    comm.as_bytes() == expected
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn owner_identity_is_the_decimal_pid_start_pair() {
        let identity = owner_identity().expect("own /proc stat is readable");
        let (pid, ticks) = identity.split_once(':').expect("pid:ticks shape");
        assert_eq!(pid, std::process::id().to_string());
        assert_eq!(
            ticks,
            process_start_time(std::process::id()).unwrap().to_string()
        );
        assert!(ticks.chars().all(|c| c.is_ascii_digit()));
    }

    fn test_pid_path(dir: &TempDir) -> PathBuf {
        dir.path().join("test.pid")
    }

    fn fake_binary_path() -> &'static Path {
        Path::new("/bin/sh")
    }

    fn fake_config_path() -> &'static Path {
        Path::new("/tmp/test-config.json")
    }

    fn spawn_backend_like_process(dir: &TempDir) -> (std::process::Child, PathBuf) {
        let script_path = dir.path().join("mock-backend.sh");
        fs::write(&script_path, "#!/bin/sh\nsleep 60\n").unwrap();
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();

        let config_path = dir.path().join("config.json");
        fs::write(&config_path, "{}").unwrap();

        let child = Command::new("/bin/sh")
            .arg(&script_path)
            .arg("run")
            .arg("-c")
            .arg(&config_path)
            .spawn()
            .unwrap();

        (child, config_path)
    }

    #[test]
    fn write_and_read_roundtrip() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));
        let test_pid = std::process::id();

        pid_file
            .write(test_pid, fake_binary_path(), fake_config_path())
            .unwrap();

        let record = pid_file.read().unwrap().unwrap();
        assert_eq!(record.pid, test_pid);
        assert_eq!(record.binary_path, canonicalize_best(fake_binary_path()));
        assert_eq!(record.config_path, canonicalize_best(fake_config_path()));

        let mode = fs::metadata(&pid_file.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    fn capability_record() -> PidOwnershipRecord {
        PidOwnershipRecord {
            pid: 4242,
            binary_path: PathBuf::from("/usr/bin/xray"),
            config_path: PathBuf::from("/run/v2ray-rs/generated/xray.json"),
            start_time_ticks: 987_654,
        }
    }

    #[test]
    fn eacces_exe_fallback_verifies_identity() {
        let record = capability_record();

        assert!(matches_record_parts(
            &record,
            ExeLookup::PermissionDenied,
            record.start_time_ticks,
            Some(&record.config_path),
            Some("xray"),
        ));
    }

    #[test]
    fn eacces_exe_fallback_rejects_mismatched_identity() {
        let record = capability_record();

        assert!(!matches_record_parts(
            &record,
            ExeLookup::PermissionDenied,
            record.start_time_ticks + 1,
            Some(&record.config_path),
            Some("xray"),
        ));
        assert!(!matches_record_parts(
            &record,
            ExeLookup::PermissionDenied,
            record.start_time_ticks,
            Some(&record.config_path),
            Some("sing-box"),
        ));
        assert!(!matches_record_parts(
            &record,
            ExeLookup::PermissionDenied,
            record.start_time_ticks,
            Some(Path::new("/run/other.json")),
            Some("xray"),
        ));
        assert!(!matches_record_parts(
            &record,
            ExeLookup::PermissionDenied,
            record.start_time_ticks,
            Some(&record.config_path),
            None,
        ));
    }

    #[test]
    fn missing_exe_never_matches_even_with_matching_fields() {
        let record = capability_record();
        assert!(!matches_record_parts(
            &record,
            ExeLookup::Missing,
            record.start_time_ticks,
            Some(&record.config_path),
            Some("xray"),
        ));
    }

    #[test]
    fn reused_pid_with_new_start_time_is_not_ours() {
        let record = capability_record();
        assert!(!matches_record_parts(
            &record,
            ExeLookup::Path(record.binary_path.clone()),
            record.start_time_ticks + 1,
            Some(&record.config_path),
            Some("xray"),
        ));
    }

    #[test]
    fn comm_matches_truncates_long_binary_names_to_fifteen_bytes() {
        let mut record = capability_record();
        record.binary_path = PathBuf::from("/usr/bin/a-very-long-backend-name");
        assert!(comm_matches("a-very-long-bac", &record));
        assert!(!comm_matches("a-very-long-back", &record));
    }

    #[test]
    fn read_nonexistent_returns_none() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        let result = pid_file.read().unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn remove_nonexistent_no_error() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        let result = pid_file.remove();
        assert!(result.is_ok());
    }

    #[test]
    fn concurrent_writes_use_distinct_temp_files() {
        use std::sync::Barrier;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("backend.pid");
        let pid = std::process::id();
        let barrier = std::sync::Arc::new(Barrier::new(4));

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let barrier = barrier.clone();
                let path = path.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..25 {
                        PidFile::new(path.clone())
                            .write(pid, Path::new("/bin/true"), Path::new("/etc/hosts"))
                            .expect("concurrent writes must not collide");
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let record = PidFile::new(path.clone()).read().unwrap().unwrap();
        assert_eq!(record.pid, pid);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn write_overwrites_existing() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        pid_file
            .write(std::process::id(), fake_binary_path(), fake_config_path())
            .unwrap();
        pid_file
            .write(
                std::process::id(),
                fake_binary_path(),
                Path::new("/tmp/other.json"),
            )
            .unwrap();

        let record = pid_file.read().unwrap().unwrap();
        assert_eq!(record.config_path, PathBuf::from("/tmp/other.json"));
    }

    #[test]
    fn write_creates_parent_directory() {
        let dir = TempDir::new().unwrap();
        let nested_path = dir.path().join("nested").join("dir").join("test.pid");
        let pid_file = PidFile::new(nested_path.clone());

        pid_file
            .write(std::process::id(), fake_binary_path(), fake_config_path())
            .unwrap();

        assert!(nested_path.exists());
        let record = pid_file.read().unwrap().unwrap();
        assert_eq!(record.pid, std::process::id());
    }

    #[test]
    fn check_and_kill_orphaned_with_no_file() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        let found_orphan = pid_file.check_and_kill_orphaned().unwrap();
        assert!(!found_orphan);
    }

    #[test]
    fn check_and_kill_orphaned_with_dead_process() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        let record = PidOwnershipRecord {
            pid: 999999,
            binary_path: fake_binary_path().into(),
            config_path: fake_config_path().into(),
            start_time_ticks: 0,
        };
        fs::write(&pid_file.path, serde_json::to_vec(&record).unwrap()).unwrap();

        let found_orphan = pid_file.check_and_kill_orphaned().unwrap();
        assert!(!found_orphan);
        assert!(pid_file.read_entry().unwrap().is_none());
    }

    #[test]
    fn check_and_kill_orphaned_cleans_up_legacy_pid_file_without_signaling() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        fs::write(&pid_file.path, "12345").unwrap();
        let found_orphan = pid_file.check_and_kill_orphaned().unwrap();

        assert!(!found_orphan);
        assert!(!pid_file.path.exists());
    }

    #[test]
    fn check_and_kill_orphaned_requires_full_record_match() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));
        let (mut child, _config_path) = spawn_backend_like_process(&dir);
        let pid = child.id();

        let record = PidOwnershipRecord {
            pid,
            binary_path: canonicalize_best(Path::new("/bin/sh")),
            config_path: canonicalize_best(Path::new("/tmp/wrong-config.json")),
            start_time_ticks: process_start_time(pid).unwrap(),
        };
        fs::write(&pid_file.path, serde_json::to_vec(&record).unwrap()).unwrap();

        let found_orphan = pid_file.check_and_kill_orphaned().unwrap();
        let still_alive = child.try_wait().unwrap().is_none();
        child.kill().ok();
        child.wait().ok();

        assert!(!found_orphan);
        assert!(
            still_alive,
            "unrelated replacement process must be left alone"
        );
        assert!(!pid_file.path.exists());
    }

    #[test]
    fn record_disposition_confirms_exit_for_each_signal_phase() {
        assert_eq!(
            record_disposition(true, None),
            RecordDisposition::DropConfirmedExit
        );
        assert_eq!(
            record_disposition(false, Some(true)),
            RecordDisposition::DropConfirmedExit
        );
        assert_eq!(
            record_disposition(true, Some(false)),
            RecordDisposition::DropConfirmedExit
        );
    }

    #[test]
    fn record_disposition_keeps_record_when_exit_never_confirms() {
        // TERM timeout followed by an unconfirmed SIGKILL: the record must
        // stay so replacement startup remains blocked.
        assert_eq!(
            record_disposition(false, Some(false)),
            RecordDisposition::KeepUnconfirmed
        );
    }

    #[test]
    fn open_disposition_maps_gone_unsupported_and_failures() {
        assert!(matches!(
            open_disposition(Some(nix::libc::ESRCH)),
            OpenFailure::Gone
        ));
        assert!(matches!(
            open_disposition(Some(nix::libc::ENOSYS)),
            OpenFailure::Unsupported
        ));
        assert!(matches!(
            open_disposition(Some(nix::libc::EINVAL)),
            OpenFailure::Failed(_)
        ));
        assert!(matches!(open_disposition(None), OpenFailure::Failed(_)));
    }

    #[test]
    fn check_and_kill_orphaned_escalates_to_sigkill() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        // A backend that ignores SIGTERM forces the SIGKILL fallback.
        let script_path = dir.path().join("stubborn-backend.sh");
        fs::write(&script_path, "#!/bin/sh\ntrap '' TERM\nsleep 60\n").unwrap();
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        let config_path = dir.path().join("config.json");
        fs::write(&config_path, "{}").unwrap();
        let mut child = Command::new("/bin/sh")
            .arg(&script_path)
            .arg("run")
            .arg("-c")
            .arg(&config_path)
            .spawn()
            .unwrap();
        let pid = child.id();
        std::thread::sleep(std::time::Duration::from_millis(100));

        pid_file
            .write(pid, Path::new("/bin/sh"), &config_path)
            .unwrap();

        let found_orphan = pid_file.check_and_kill_orphaned().unwrap();
        let status = child.wait().unwrap();

        assert!(found_orphan);
        assert!(!status.success());
        assert!(!process_exists(pid));
        assert!(!pid_file.path.exists());
    }

    #[test]
    fn check_and_kill_orphaned_matching_record_terminates_process() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));
        let (mut child, config_path) = spawn_backend_like_process(&dir);
        let pid = child.id();

        std::thread::sleep(std::time::Duration::from_millis(100));

        pid_file
            .write(pid, Path::new("/bin/sh"), &config_path)
            .unwrap();

        let found_orphan = pid_file.check_and_kill_orphaned().unwrap();
        let status = child.wait().unwrap();

        assert!(found_orphan);
        assert!(!status.success());
        assert!(!pid_file.path.exists());
    }

    #[test]
    fn read_invalid_pid_returns_error() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        fs::write(&pid_file.path, "not_a_record").unwrap();

        let result = pid_file.read();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
