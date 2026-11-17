use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};

fn canonicalize_best(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn io_error_from_errno(err: Errno) -> io::Error {
    io::Error::from_raw_os_error(err as i32)
}

fn process_exists(pid: u32) -> bool {
    matches!(
        kill(Pid::from_raw(pid as i32), None),
        Ok(()) | Err(Errno::EPERM)
    )
}

enum ExeLookup {
    Path(PathBuf),
    PermissionDenied,
    Missing,
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

fn process_start_time(pid: u32) -> io::Result<u64> {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidOwnershipRecord {
    pub pid: u32,
    pub binary_path: PathBuf,
    pub config_path: PathBuf,
    pub start_time_ticks: u64,
}

fn atomic_write_0600(path: &Path, payload: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("backend.pid");
    let tmp = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));

    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(payload)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

enum PidFileEntry {
    Record(PidOwnershipRecord),
    Legacy,
    Corrupt,
}

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

        if !process_exists(record.pid) {
            self.remove()?;
            return Ok(false);
        }

        if !process_matches_record(&record) {
            self.remove()?;
            return Ok(false);
        }

        match kill(Pid::from_raw(record.pid as i32), Signal::SIGTERM) {
            Ok(()) => {
                for _ in 0..5 {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    if !process_exists(record.pid) {
                        break;
                    }
                }
                if process_exists(record.pid) {
                    // The 1s wait is a window for PID reuse: re-verify the
                    // recorded start time immediately before escalating, or an
                    // unrelated process could eat the SIGKILL. An unreadable
                    // start time counts as changed.
                    if process_start_time(record.pid)
                        .map(|start| start != record.start_time_ticks)
                        .unwrap_or(true)
                    {
                        self.remove()?;
                        return Ok(false);
                    }
                    let _ = kill(Pid::from_raw(record.pid as i32), Signal::SIGKILL);
                    for _ in 0..5 {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        if !process_exists(record.pid) {
                            break;
                        }
                    }
                }
                self.remove()?;
                Ok(true)
            }
            Err(Errno::ESRCH) => {
                self.remove()?;
                Ok(false)
            }
            Err(err) => Err(io_error_from_errno(err)),
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
        child.kill().ok();
        child.wait().ok();

        assert!(!found_orphan);
        assert!(!pid_file.path.exists());
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
