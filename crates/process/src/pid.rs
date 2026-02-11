use std::fs;
use std::path::PathBuf;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

pub struct PidFile {
    path: PathBuf,
}

impl PidFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn write(&self, pid: u32) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, pid.to_string())
    }

    pub fn read(&self) -> std::io::Result<Option<u32>> {
        match fs::read_to_string(&self.path) {
            Ok(content) => {
                let pid = content.trim().parse::<u32>()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(Some(pid))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn remove(&self) -> std::io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn check_and_kill_orphaned(&self) -> std::io::Result<bool> {
        let Some(pid) = self.read()? else {
            return Ok(false);
        };

        let nix_pid = Pid::from_raw(pid as i32);

        let process_exists = kill(nix_pid, None).is_ok();

        if process_exists {
            let _ = kill(nix_pid, Signal::SIGTERM);
            self.remove()?;
            Ok(true)
        } else {
            self.remove()?;
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_pid_path(dir: &TempDir) -> PathBuf {
        dir.path().join("test.pid")
    }

    #[test]
    fn write_and_read_roundtrip() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        let test_pid = 12345u32;
        pid_file.write(test_pid).unwrap();

        let read_pid = pid_file.read().unwrap();
        assert_eq!(read_pid, Some(test_pid));
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

        pid_file.write(11111).unwrap();
        pid_file.write(22222).unwrap();

        let read_pid = pid_file.read().unwrap();
        assert_eq!(read_pid, Some(22222));
    }

    #[test]
    fn write_creates_parent_directory() {
        let dir = TempDir::new().unwrap();
        let nested_path = dir.path().join("nested").join("dir").join("test.pid");
        let pid_file = PidFile::new(nested_path.clone());

        pid_file.write(99999).unwrap();

        assert!(nested_path.exists());
        let read_pid = pid_file.read().unwrap();
        assert_eq!(read_pid, Some(99999));
    }

    #[test]
    fn check_and_kill_orphaned_with_no_file() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        let found_orphan = pid_file.check_and_kill_orphaned().unwrap();
        assert_eq!(found_orphan, false);
    }

    #[test]
    fn check_and_kill_orphaned_with_dead_process() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        let dead_pid = 999999u32;
        pid_file.write(dead_pid).unwrap();

        let found_orphan = pid_file.check_and_kill_orphaned().unwrap();
        assert_eq!(found_orphan, false);

        let read_result = pid_file.read().unwrap();
        assert_eq!(read_result, None);
    }

    #[test]
    fn check_and_kill_orphaned_cleans_up_pid_file() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        let fake_pid = 999999u32;
        pid_file.write(fake_pid).unwrap();

        pid_file.check_and_kill_orphaned().unwrap();

        let read_result = pid_file.read().unwrap();
        assert_eq!(read_result, None);
    }

    #[test]
    fn read_invalid_pid_returns_error() {
        let dir = TempDir::new().unwrap();
        let pid_file = PidFile::new(test_pid_path(&dir));

        fs::write(&pid_file.path, "not_a_number").unwrap();

        let result = pid_file.read();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    }
}
