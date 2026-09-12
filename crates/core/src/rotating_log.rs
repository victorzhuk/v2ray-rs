//! Append-only, size-rotated log file for backend diagnostics records.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
///
/// The lock lives inside the writer: `&self` methods are safe to share
/// across threads via `Arc<RotatingFileWriter>`.
pub struct RotatingFileWriter {
    path: PathBuf,
    max_bytes: u64,
    inner: Mutex<Inner>,
}

struct Inner {
    file: Option<fs::File>,
    bytes: u64,
    warned: bool,
    /// True once the destructive chain has retired the active file to `.1`
    /// but the fresh open failed. While set, subsequent rotates only retry
    /// the open — they MUST NOT repeat the rename chain, which would shift
    /// `x.log.1` → `x.log.2` again and corrupt the rotation set.
    pending_reopen: bool,
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
            inner: Mutex::new(Inner {
                file: Some(file),
                bytes,
                warned: false,
                pending_reopen: false,
            }),
        })
    }

    /// Appends one record prefixed with an RFC 3339 timestamp and a trailing
    /// newline. Equivalent to `append_line("", content)`.
    pub fn append(&self, content: &str) {
        self.append_line("", content);
    }

    /// Appends one record prefixed with an RFC 3339 timestamp, an optional
    /// stream tag, and a trailing newline. The on-disk shape is
    /// `<rfc3339> [<stream>] <content>\n`; an empty `stream` omits the tag.
    pub fn append_line(&self, stream: &str, content: &str) {
        let timestamp = Utc::now().to_rfc3339();
        let line = if stream.is_empty() {
            format!("{timestamp} {content}\n")
        } else {
            format!("{timestamp} {stream} {content}\n")
        };

        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let inner = &mut *guard;

        let needed = line.len() as u64;
        if inner.bytes + needed > self.max_bytes {
            Self::rotate(&self.path, inner);
        }

        let Some(file) = inner.file.as_mut() else {
            return;
        };

        match file.write_all(line.as_bytes()) {
            Ok(()) => inner.bytes += needed,
            Err(e) => Self::report(&self.path, &mut inner.warned, "write", e),
        }
    }

    /// Bails at the first non-NotFound filesystem error and only resets the
    /// byte counter after the active file was successfully retired AND a
    /// replacement opened; that keeps a partial rotation from leaving the
    /// writer with a missing file handle and a wrong offset. `append` stays
    /// nonfatal: a failed rotation drops the active handle, so subsequent
    /// writes are silently skipped until the next append re-triggers a
    /// rotation that can succeed.
    ///
    /// Once the destructive chain has retired the active file (renamed to
    /// `.1`) but the fresh open fails, `pending_reopen` is set and later
    /// threshold crossings skip the rename chain entirely — they only retry
    /// the open. This stops a reopen failure after retirement from causing
    /// `x.log.1` to shift to `x.log.2` on every subsequent append.
    fn rotate(path: &Path, inner: &mut Inner) {
        if inner.pending_reopen {
            // Active file is already retired at `.1`; only retry the open.
            // The destructive chain must not run again — that would shift
            // x.log.1 → x.log.2 a second time.
            match Self::open_file(path) {
                Ok(file) => {
                    inner.file = Some(file);
                    inner.bytes = 0;
                    inner.pending_reopen = false;
                }
                Err(e) => Self::report(path, &mut inner.warned, "rotate: reopen", e),
            }
            return;
        }

        inner.file = None;

        let oldest = rotated_path(path, ROTATIONS_KEPT);
        if let Err(e) = fs::remove_file(&oldest)
            && e.kind() != io::ErrorKind::NotFound
        {
            Self::report(path, &mut inner.warned, "rotate: remove oldest", e);
            return;
        }

        for generation in (1..ROTATIONS_KEPT).rev() {
            let from = rotated_path(path, generation);
            let to = rotated_path(path, generation + 1);
            if let Err(e) = fs::rename(&from, &to)
                && e.kind() != io::ErrorKind::NotFound
            {
                Self::report(path, &mut inner.warned, "rotate: shift", e);
                return;
            }
        }

        if let Err(e) = fs::rename(path, rotated_path(path, 1))
            && e.kind() != io::ErrorKind::NotFound
        {
            Self::report(path, &mut inner.warned, "rotate: retire active", e);
            return;
        }

        // Retirement succeeded; mark the writer so the next rotate does not
        // re-run the rename chain. The reopen below either clears the flag
        // (success) or leaves it set with `inner.file = None` (failure → the
        // next append will retry only the open).
        inner.pending_reopen = true;
        match Self::open_file(path) {
            Ok(file) => {
                inner.file = Some(file);
                inner.bytes = 0;
                inner.pending_reopen = false;
            }
            Err(e) => Self::report(path, &mut inner.warned, "rotate: reopen", e),
        }
    }

    fn open_file(path: &Path) -> io::Result<fs::File> {
        fs::OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(path)
    }

    fn report(path: &Path, warned: &mut bool, what: &str, error: io::Error) {
        if !*warned {
            *warned = true;
            eprintln!(
                "rotating log {}: {what} failed, giving up: {error}",
                path.display()
            );
        }
    }
}

#[cfg(test)]
impl RotatingFileWriter {
    /// Test-only constructor that places the writer in the post-retirement,
    /// reopen-failed state. The destructive chain is treated as already
    /// complete, so the very next threshold crossing must skip the rename
    /// chain and retry only the open.
    pub(crate) fn _with_pending_reopen_for_test(path: PathBuf, max_bytes: u64) -> Self {
        Self {
            path,
            max_bytes,
            inner: Mutex::new(Inner {
                file: None,
                bytes: u64::MAX / 2,
                warned: false,
                pending_reopen: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn line_len() -> u64 {
        // RFC 3339 timestamp (<= 35 chars) + separator + stream tag +
        // separator + 48-char marker + newline.
        35 + 1 + 1 + 1 + 48 + 1
    }

    #[test]
    fn rotating_writer_rotates_at_threshold() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("x.log");
        let max_bytes = 200;
        let writer = RotatingFileWriter::open(&path, max_bytes).unwrap();

        let marker = "A".repeat(48);
        writer.append_line("m", &marker);
        writer.append_line("m", &marker);

        assert!(!rotated_path(&path, 1).exists());

        writer.append_line("m", &marker);

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
        let writer = RotatingFileWriter::open(&path, 64).unwrap();

        for marker in ["m1", "m2", "m3", "m4", "m5", "m6"] {
            writer.append_line("m", marker);
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
        let writer = RotatingFileWriter::open(&path, DEFAULT_MAX_BYTES).unwrap();
        writer.append_line("p", "secret");
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
            let writer = RotatingFileWriter::open(&path, DEFAULT_MAX_BYTES).unwrap();
            writer.append_line("p", "first");
        }
        {
            let writer = RotatingFileWriter::open(&path, DEFAULT_MAX_BYTES).unwrap();
            writer.append_line("p", "second");
        }

        let content = fs::read_to_string(&path).unwrap();
        let mut lines = content.lines();
        for expected in ["first", "second"] {
            let line = lines.next().unwrap();
            let (timestamp, rest) = line.split_once(' ').unwrap();
            chrono::DateTime::parse_from_rfc3339(timestamp)
                .expect("record must carry an RFC 3339 timestamp");
            assert!(
                rest.ends_with(expected),
                "expected {expected:?} at end of {rest:?}"
            );
        }
        assert!(lines.next().is_none());
    }

    #[test]
    fn rotating_writer_survives_rotate_failure() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("x.log");
        // Block the rotation at its first step: a directory at x.log.3 makes the
        // oldest-generation removal fail deterministically, so rotate() bails
        // before touching the active file.
        std::fs::create_dir(rotated_path(&path, 3)).unwrap();

        let writer = RotatingFileWriter::open(&path, 200).unwrap();
        let marker = "A".repeat(48);
        writer.append_line("m", &marker);
        writer.append_line("m", &marker);
        // Third write crosses the threshold; removing x.log.3 hits the directory
        // and rotate() bails, dropping this record. Further appends must not
        // panic even though no bytes can land until the blocker clears.
        writer.append_line("m", &marker);
        writer.append_line("m", &marker);

        // The failed rotation left the active file and its records untouched.
        let survivors = fs::read_to_string(&path).unwrap();
        assert!(
            survivors.contains(&marker),
            "records written before the failed rotation must survive: {survivors}"
        );
        assert_eq!(survivors.lines().count(), 2, "{survivors}");
        assert!(
            !rotated_path(&path, 1).exists(),
            "a failed rotation must not retire the active file"
        );

        // Clearing the blocker lets the same writer recover: the next threshold
        // crossing rotates, the survivors move to x.log.1, and appends land again.
        std::fs::remove_dir(rotated_path(&path, 3)).unwrap();
        writer.append_line("m", "recovered");

        let rotated = fs::read_to_string(rotated_path(&path, 1)).unwrap();
        assert!(
            rotated.contains(&marker),
            "survivors must be retired to x.log.1 once rotation succeeds: {rotated}"
        );
        writer.append_line("m", "post");
        let content = fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("recovered") && content.contains("post"),
            "the same writer must accept appends after recovery: {content}"
        );
    }

    #[test]
    fn rotating_writer_pending_reopen_skips_destructive_shift() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("x.log");

        // Recreate the state right after a successful retirement + failed
        // reopen: x.log.1 holds the just-retired active file, x.log.2 and
        // x.log.3 hold earlier generations. A directory at the active path
        // keeps the reopen failing (append on a directory is an error), which
        // is what makes the pending state real; while it holds, the
        // destructive chain must not re-run — that would shift x.log.1 →
        // x.log.2 again and corrupt the rotation set.
        fs::write(rotated_path(&path, 1), b"retired-active\n").unwrap();
        fs::write(rotated_path(&path, 2), b"sentinel.2\n").unwrap();
        fs::write(rotated_path(&path, 3), b"sentinel.3\n").unwrap();
        fs::create_dir(&path).unwrap();

        let s1_content = fs::read_to_string(rotated_path(&path, 1)).unwrap();
        let s2_content = fs::read_to_string(rotated_path(&path, 2)).unwrap();
        let s3_content = fs::read_to_string(rotated_path(&path, 3)).unwrap();
        let s1_mtime = fs::metadata(rotated_path(&path, 1))
            .unwrap()
            .modified()
            .unwrap();
        let s2_mtime = fs::metadata(rotated_path(&path, 2))
            .unwrap()
            .modified()
            .unwrap();
        let s3_mtime = fs::metadata(rotated_path(&path, 3))
            .unwrap()
            .modified()
            .unwrap();

        let writer = RotatingFileWriter::_with_pending_reopen_for_test(path.clone(), 64);

        // Records that would normally trigger another rotate must be dropped
        // silently while the reopen keeps failing: nothing may land anywhere,
        // and the rotation set must not shift.
        let big = "B".repeat(96);
        writer.append_line("m", &big);
        writer.append_line("m", &big);

        assert_eq!(
            fs::read_to_string(rotated_path(&path, 1)).unwrap(),
            s1_content,
            "x.log.1 must not be shifted while a reopen is pending",
        );
        assert_eq!(
            fs::read_to_string(rotated_path(&path, 2)).unwrap(),
            s2_content,
            "x.log.2 must not be re-shifted while a reopen is pending",
        );
        assert_eq!(
            fs::read_to_string(rotated_path(&path, 3)).unwrap(),
            s3_content,
            "x.log.3 must not be re-shifted while a reopen is pending",
        );
        assert_eq!(
            fs::metadata(rotated_path(&path, 1))
                .unwrap()
                .modified()
                .unwrap(),
            s1_mtime,
            "x.log.1 mtime must not change while a reopen is pending",
        );
        assert_eq!(
            fs::metadata(rotated_path(&path, 2))
                .unwrap()
                .modified()
                .unwrap(),
            s2_mtime,
            "x.log.2 mtime must not change while a reopen is pending",
        );
        assert_eq!(
            fs::metadata(rotated_path(&path, 3))
                .unwrap()
                .modified()
                .unwrap(),
            s3_mtime,
            "x.log.3 mtime must not change while a reopen is pending",
        );
        assert!(
            fs::read_dir(&path).unwrap().next().is_none(),
            "blocked appends must not write records anywhere",
        );

        // Removing the obstruction lets the next threshold crossing succeed:
        // the pending flag clears, a fresh x.log receives the record, and the
        // rotation set stays untouched by the recovery.
        fs::remove_dir(&path).unwrap();
        writer.append_line("m", &big);
        let active = fs::read_to_string(&path).unwrap();
        assert!(
            active.contains(&big),
            "active file must receive records once reopen succeeds: {active}",
        );
        assert_eq!(
            fs::read_to_string(rotated_path(&path, 1)).unwrap(),
            s1_content,
            "recovery must not shift the rotation set",
        );

        // With the flag cleared, a further threshold crossing rotates
        // normally: the recovered active file retires to x.log.1 and the set
        // shifts exactly one generation.
        writer.append_line("m", "post");
        assert!(
            fs::read_to_string(rotated_path(&path, 1))
                .unwrap()
                .contains(&big),
            "recovered active file must retire to x.log.1 on a normal rotation",
        );
        assert_eq!(
            fs::read_to_string(rotated_path(&path, 2)).unwrap(),
            s1_content,
            "normal rotation must shift the set exactly one generation",
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap().lines().count(),
            1,
            "fresh active file must hold only the post-recovery record",
        );
    }
}
