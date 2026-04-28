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

fn process_binary_path(pid: u32) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{pid}/exe")).ok()
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
        fs::write(&self.path, payload)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
        }

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
    let Some(actual_binary_path) = process_binary_path(record.pid) else {
        return false;
    };
    let Some(actual_config_path) = process_config_path(record.pid) else {
        return false;
    };
    let Ok(actual_start_time) = process_start_time(record.pid) else {
        return false;
    };

    canonicalize_best(&actual_binary_path) == record.binary_path
        && canonicalize_best(&actual_config_path) == record.config_path
        && actual_start_time == record.start_time_ticks
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
