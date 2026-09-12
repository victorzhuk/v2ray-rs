//! Append-only, size-rotated log file for backend diagnostics records.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::persistence::{PersistenceError, create_dir_with_permissions};

/// Rotation threshold: 5 MiB.
pub const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// Number of rotated files kept beside the active log: `x.log.1` … `x.log.N`,
/// where `x.log.1` is the most recent.
pub const ROTATIONS_KEPT: u32 = 3;

/// Append-only log writer that rotates at a byte threshold. Every record is
/// prefixed with an RFC 3339 timestamp and terminated with a newline. When a
/// record would push the active file past `max_bytes`, the active file is
/// rotated: the oldest generation is deleted, `x.log.N` shifts to
/// `x.log.N+1`, the active file becomes `x.log.1`, and a fresh file is
/// opened. Write and rotation failures are reported to stderr once and then
/// ignored, so `append` never fails, panics, or retries.
pub struct RotatingFileWriter {
    path: PathBuf,
    max_bytes: u64,
    file: Option<fs::File>,
    bytes: u64,
    warned: bool,
}

fn rotated_path(base: &Path, generation: u32) -> PathBuf {
    let mut name = base.as_os_str().to_owned();
    name.push(format!(".{generation}"));
    PathBuf::from(name)
}

impl RotatingFileWriter {
    /// Opens (or creates) the log file at `path`, creating parent directories
    /// with 0o700 and the file itself with 0o600. Existing content is kept and
    /// appended to.
    pub fn open(path: impl Into<PathBuf>, max_bytes: u64) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            create_dir_with_permissions(parent).map_err(|e| match e {
                PersistenceError::Io(io) => io,
                other => io::Error::other(other.to_string()),
            })?;
        }

        let file = Self::open_file(&path)?;
        let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);

        Ok(Self {
            path,
            max_bytes,
            file: Some(file),
            bytes,
            warned: false,
        })
    }

    /// Appends one record, prefixed with an RFC 3339 timestamp; a trailing
    /// newline is added when the record lacks one.
    pub fn append(&mut self, record: &[u8]) {
        let timestamp = Utc::now().to_rfc3339();
        let needs_newline = record.last() != Some(&b'\n');
        let needed = (timestamp.len() + 1 + record.len() + usize::from(needs_newline)) as u64;

        if self.bytes + needed > self.max_bytes {
            self.rotate();
        }

        let Some(file) = self.file.as_mut() else {
            return;
        };

        let mut line = Vec::with_capacity(needed as usize);
        line.extend_from_slice(timestamp.as_bytes());
        line.push(b' ');
        line.extend_from_slice(record);
        if needs_newline {
            line.push(b'\n');
        }

        match file.write_all(&line) {
            Ok(()) => self.bytes += needed,
            Err(e) => self.report("write", e),
        }
    }

    /// Appends one line, prefixed with an RFC 3339 timestamp.
    pub fn append_line(&mut self, line: &str) {
        self.append(line.as_bytes());
    }

    fn rotate(&mut self) {
        self.file = None;

        let oldest = rotated_path(&self.path, ROTATIONS_KEPT);
        if let Err(e) = fs::remove_file(&oldest)
            && e.kind() != io::ErrorKind::NotFound
        {
            self.report("rotate: remove oldest", e);
        }

        for generation in (1..ROTATIONS_KEPT).rev() {
            let from = rotated_path(&self.path, generation);
            let to = rotated_path(&self.path, generation + 1);
            if let Err(e) = fs::rename(&from, &to)
                && e.kind() != io::ErrorKind::NotFound
            {
                self.report("rotate: shift", e);
            }
        }

        if let Err(e) = fs::rename(&self.path, rotated_path(&self.path, 1))
            && e.kind() != io::ErrorKind::NotFound
        {
            self.report("rotate: retire active", e);
        }

        match Self::open_file(&self.path) {
            Ok(file) => {
                self.file = Some(file);
                self.bytes = 0;
            }
            Err(e) => self.report("rotate: reopen", e),
        }
    }

    fn open_file(path: &Path) -> io::Result<fs::File> {
        fs::OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(path)
    }

    fn report(&mut self, what: &str, error: io::Error) {
        if !self.warned {
            self.warned = true;
            eprintln!(
                "rotating log {}: {what} failed, giving up: {error}",
                self.path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn line_len() -> u64 {
        // RFC 3339 timestamp (<= 35 chars) + separator + marker + newline.
        35 + 1 + 48 + 1
    }

    #[test]
    fn rotating_writer_rotates_at_threshold() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("x.log");
        let max_bytes = 200;
        let mut writer = RotatingFileWriter::open(&path, max_bytes).unwrap();

        let marker = "A".repeat(48);
        writer.append_line(&marker);
        writer.append_line(&marker);

        assert!(!rotated_path(&path, 1).exists());

        writer.append_line(&marker);

        assert!(rotated_path(&path, 1).exists());
        assert_eq!(
            fs::read_to_string(rotated_path(&path, 1))
                .unwrap()
                .lines()
                .count(),
            2
        );
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 1);
        assert!(fs::metadata(&path).unwrap().len() <= max_bytes);
        assert!(fs::metadata(rotated_path(&path, 1)).unwrap().len() <= max_bytes);
        assert!(fs::metadata(&path).unwrap().len() <= line_len());
    }

    #[test]
    fn rotating_writer_keeps_at_most_three_rotations() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("x.log");
        let mut writer = RotatingFileWriter::open(&path, 64).unwrap();

        for marker in ["m1", "m2", "m3", "m4", "m5", "m6"] {
            writer.append_line(marker);
        }

        let mut entries: Vec<String> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        entries.sort();
        assert_eq!(entries, ["x.log", "x.log.1", "x.log.2", "x.log.3"]);

        let kept: String = ["x.log", "x.log.1", "x.log.2", "x.log.3"]
            .iter()
            .map(|name| fs::read_to_string(tmp.path().join(name)).unwrap())
            .collect();
        assert!(!kept.contains("m1"));
        assert!(!kept.contains("m2"));
        assert!(kept.contains("m3"));
        assert!(
            fs::read_to_string(rotated_path(&path, 3))
                .unwrap()
                .contains("m3")
        );
    }

    #[test]
    fn rotating_writer_creates_private_file_and_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("nested/sub");
        let path = dir.join("x.log");
        let mut writer = RotatingFileWriter::open(&path, DEFAULT_MAX_BYTES).unwrap();
        writer.append_line("secret");
        drop(writer);

        let dir_mode = fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700);

        let file_mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(file_mode & 0o777, 0o600);
    }

    #[test]
    fn rotating_writer_prepends_timestamp_and_appends_across_opens() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("x.log");

        {
            let mut writer = RotatingFileWriter::open(&path, DEFAULT_MAX_BYTES).unwrap();
            writer.append_line("first");
        }
        {
            let mut writer = RotatingFileWriter::open(&path, DEFAULT_MAX_BYTES).unwrap();
            writer.append_line("second");
        }

        let content = fs::read_to_string(&path).unwrap();
        let mut lines = content.lines();
        for expected in ["first", "second"] {
            let line = lines.next().unwrap();
            let timestamp = line.strip_suffix(expected).unwrap().trim_end();
            chrono::DateTime::parse_from_rfc3339(timestamp).unwrap();
        }
        assert!(lines.next().is_none());
    }
}
