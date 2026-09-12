//! Application logging: a size-rotated file under the state logs directory,
//! mirrored to stderr. Installed once at startup before any persistence call,
//! so early persistence failures are captured; an unwritable log file degrades
//! to stderr-only and never blocks startup.

use std::io;
use std::path::PathBuf;

use log::{LevelFilter, Log, Metadata, Record};
use v2ray_rs_core::persistence::AppPaths;
use v2ray_rs_core::rotating_log::{DEFAULT_MAX_BYTES, RotatingFileWriter};

const LEVEL_ENV: &str = "V2RAY_RS_LOG";
const LOG_FILE_NAME: &str = "v2ray-rs.log";

pub struct AppLogger {
    writer: Option<RotatingFileWriter>,
    max_level: LevelFilter,
}

impl AppLogger {
    fn new(path: PathBuf, max_level: LevelFilter) -> io::Result<Self> {
        let writer = RotatingFileWriter::open(path, DEFAULT_MAX_BYTES)?;
        Ok(Self {
            writer: Some(writer),
            max_level,
        })
    }

    fn stderr_only(max_level: LevelFilter) -> Self {
        Self {
            writer: None,
            max_level,
        }
    }
}

impl Log for AppLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.max_level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!("{} {} {}", record.level(), record.target(), record.args())
            .replace(['\r', '\n'], " ");
        eprintln!("{} {}", chrono::Utc::now().to_rfc3339(), line);
        if let Some(writer) = self.writer.as_ref() {
            writer.append(&line);
        }
    }

    fn flush(&self) {}
}

fn parse_level(value: &str) -> Option<LevelFilter> {
    match value.trim().to_ascii_lowercase().as_str() {
        "trace" => Some(LevelFilter::Trace),
        "debug" => Some(LevelFilter::Debug),
        "info" => Some(LevelFilter::Info),
        "warn" => Some(LevelFilter::Warn),
        "error" => Some(LevelFilter::Error),
        _ => None,
    }
}

pub(crate) fn resolve_level(value: Option<&str>) -> LevelFilter {
    value.and_then(parse_level).unwrap_or(LevelFilter::Info)
}

pub fn init_logging(paths: &AppPaths) {
    let raw = std::env::var(LEVEL_ENV).ok();
    let max_level = resolve_level(raw.as_deref());
    if let Some(value) = raw.as_deref()
        && parse_level(value).is_none()
    {
        eprintln!(
            "v2ray-rs: ignoring invalid {LEVEL_ENV} value {value:?}, using 'info' \
             (accepted: trace, debug, info, warn, error)"
        );
    }

    let log_path = paths.logs_dir().join(LOG_FILE_NAME);
    let logger = match AppLogger::new(log_path.clone(), max_level) {
        Ok(logger) => logger,
        Err(err) => {
            eprintln!(
                "v2ray-rs: cannot open {}: {err}, logging to stderr only",
                log_path.display()
            );
            AppLogger::stderr_only(max_level)
        }
    };

    // Fails only when a logger is already installed; the first one stays.
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(max_level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Level;
    use std::fs;
    use tempfile::TempDir;

    fn log_record(logger: &AppLogger, level: Level, message: &str) {
        Log::log(
            logger,
            &Record::builder()
                .level(level)
                .target("test-target")
                .args(format_args!("{message}"))
                .build(),
        );
    }

    #[test]
    fn app_logger_writes_warn_and_filters_debug_by_default() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(LOG_FILE_NAME);
        let logger = AppLogger::new(path.clone(), LevelFilter::Info).unwrap();

        log_record(&logger, Level::Warn, "beware");
        log_record(&logger, Level::Debug, "quiet");

        let content = fs::read_to_string(&path).unwrap();
        let mut lines = content.lines();
        let line = lines.next().unwrap();
        assert!(
            lines.next().is_none(),
            "debug record must not reach the file at info level"
        );

        let (timestamp, rest) = line.split_once(' ').unwrap();
        chrono::DateTime::parse_from_rfc3339(timestamp)
            .expect("record must carry exactly one RFC 3339 timestamp from the writer");
        assert_eq!(rest, "WARN test-target beware");
    }

    #[test]
    fn app_logger_scrubs_newlines_from_record_text() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(LOG_FILE_NAME);
        let logger = AppLogger::new(path.clone(), LevelFilter::Info).unwrap();

        log_record(&logger, Level::Warn, "evil\nforge\r\nmore");

        let content = fs::read_to_string(&path).unwrap();
        let mut lines = content.lines();
        let (timestamp, rest) = lines.next().unwrap().split_once(' ').unwrap();
        assert!(
            lines.next().is_none(),
            "a record containing a newline must yield exactly one file line"
        );
        chrono::DateTime::parse_from_rfc3339(timestamp)
            .expect("record must carry exactly one RFC 3339 timestamp from the writer");
        assert_eq!(rest, "WARN test-target evil forge  more");
    }

    #[test]
    fn log_level_resolves_v2ray_rs_log() {
        assert_eq!(resolve_level(None), LevelFilter::Info);
        assert_eq!(resolve_level(Some("debug")), LevelFilter::Debug);
        assert_eq!(resolve_level(Some("WARN")), LevelFilter::Warn);
        assert_eq!(resolve_level(Some("Trace")), LevelFilter::Trace);
        assert_eq!(resolve_level(Some("bogus")), LevelFilter::Info);
        assert_eq!(resolve_level(Some("")), LevelFilter::Info);
    }
}
