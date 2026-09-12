# Design: two rotating files, no new dependencies

## Context

- `AppPaths::state_dir` resolves to `$XDG_STATE_HOME/<qualifier>` or `<data_dir>/state` (`crates/core/src/persistence/mod.rs:89`), per profile, so dev and production logs are already separated. Directories are created 0700.
- Backend lines are read in two tokio tasks per process (`crates/process/src/manager.rs:454`), pushed to the `LogBuffer` and a broadcast channel. The UI forwarder drops lines on broadcast lag, so it is not a lossless tap.
- The workspace depends on `log` 0.4 only; nothing implements `log::Log`.
- Real Delay probes spawn short-lived backends through `probe.rs`, separate from `ProcessManager`.

## Goals / Non-Goals

**Goals:**
- A crash's exit status and last output survive the app.
- Every application log record at `info` and above is on disk.
- Disk use is bounded without user action.

**Non-Goals:**
- A log viewer for files, export, or an "open log folder" action.
- Persisting Real Delay probe backends' output.
- Structured or JSON logs.

## Decisions

- **Files.** `<state_dir>/logs/v2ray-rs.log` for application records, `<state_dir>/logs/backend.log` for backend output. Separate files keep a chatty backend from rotating away the application's warnings.
- **Rotation in-house.** A small writer in `crates/core`: append-only, checks size before each write, at 5 MiB renames `x.log.2 → x.log.3`, `x.log.1 → x.log.2`, `x.log → x.log.1` and reopens; three rotated files kept. Created 0600 in a 0700 directory. Alternatives rejected: `flexi_logger`/`tracing-appender` — a dependency for fifty lines; `env_logger` — stderr only.
- **Logger.** A `log::Log` implementation in the UI crate writing `RFC 3339 timestamp, level, target, message` to the application file and stderr. Default level `info`; `V2RAY_RS_LOG=debug` (or `trace`, `warn`, `error`) overrides it. Installed first in `main`, before persistence initialization, so early failures are captured.
- **Backend writes at the source.** `ProcessManager::with_log_file` hands the reader tasks a shared writer; each line is written as `timestamp stream line` in the same task that fills the buffer, so nothing is lost to broadcast lag. Session and exit records are written by the manager around spawn and exit handling.
- **Failure toasts, once per streak.** The geodata service and the subscription auto-update keep a "last attempt failed" flag; a failure toasts only on the transition from success (or startup) to failure, and a later success clears it.
- **No secrets by construction.** Application records log subscription names and node labels, never URLs or credentials; the audit task checks every existing `log::` call site. Backend output is written as the backend printed it; xray at `loglevel: warning` prints destinations and errors, not credentials.

## Risks / Trade-offs

- [A write error on a full disk stalls logging] → write failures are ignored after the first, which is reported to stderr; logging never blocks or fails a connection.
- [Backend output can include destination hostnames] → the file is private to the user, same trust boundary as the generated config that already holds credentials.
- [Size check per write adds a `metadata` call] → the writer tracks its own byte count after open instead of re-stating the file.

## Migration Plan

New files only; nothing to migrate. Rollback is a revert; stale log files are harmless.

## Plan appendix

```json
{
  "v": 2,
  "change": "persist-backend-diagnostics",
  "baseSha": "b04681e811dd89e324cba966603efb02e1181c97",
  "generatedAt": "2026-09-11T21:35:03.412Z",
  "tier": "heavy",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality",
    "sec"
  ],
  "chunks": [
    {
      "id": "core-writer",
      "taskIds": [
        "1.1",
        "1.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "core-rotating-writer-paths",
      "shard": "core",
      "pkgDirs": [
        "crates/core/src"
      ],
      "pkgs": [],
      "coder": "rust-coder",
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/rotating_log.rs",
          "symbol": "RotatingFileWriter",
          "anchor": "pub mod persistence;",
          "change": "new file: append-only rotating writer — append/append_line prepend the single <rfc3339> timestamp to every line (UI logger and backend records rely on it); byte-counted 5 MiB threshold, three rotations, 0600 file via 0700 dir, self-counted bytes, infallible append; declared as pub mod rotating_log; in crates/core/src/lib.rs after persistence"
        },
        {
          "task": "1.2",
          "file": "crates/core/src/persistence/mod.rs",
          "symbol": "AppPaths::logs_dir",
          "anchor": "pub fn latency_snapshot_path(&self) -> PathBuf {",
          "change": "add pub fn logs_dir() -> PathBuf { self.state_dir.join(\"logs\") } beside latency_snapshot_path; create it in ensure_dirs via create_dir_with_permissions"
        }
      ],
      "contract": {
        "states": [
          "fresh-open",
          "under-threshold",
          "threshold-crossed",
          "rotated-3-deep"
        ],
        "transitions": [
          {
            "input": "RotatingFileWriter::open(path, max_bytes) on new or existing file",
            "state": "fresh-open",
            "effect": "set",
            "evidence": "task 1.1; R3 permissions scenario; state_dir base persistence/mod.rs:98-102"
          },
          {
            "input": "append/append_line while bytes + line + 1 <= max_bytes",
            "state": "under-threshold",
            "effect": "set",
            "evidence": "task 1.1 byte-counted threshold; design D2"
          },
          {
            "input": "append/append_line while bytes + line + 1 > max_bytes",
            "state": "threshold-crossed",
            "effect": "forced",
            "evidence": "task 1.1; design D2 rotate-then-reopen; R3 rotation scenario"
          },
          {
            "input": "rotation when x.log.1 and x.log.2 already exist",
            "state": "rotated-3-deep",
            "effect": "forced",
            "evidence": "task 1.1 three rotations, deletion of oldest; R3 at most three rotated files"
          },
          {
            "input": "write io error (full disk, removed dir)",
            "state": "under-threshold",
            "effect": "no-op",
            "evidence": "design risk line: write failures ignored after the first, reported to stderr"
          }
        ],
        "forbidden": [
          "any log file larger than max_bytes plus one line length",
          "a fourth rotated file (x.log.4) or an x.log.3 surviving the next rotation instead of being deleted",
          "log directory mode wider than 0o700 or file mode wider than 0o600 at creation",
          "append returning Err, panicking, or blocking on a retry loop"
        ],
        "seeding": [
          "fresh-open: only via RotatingFileWriter::open on a path inside a tempfile::TempDir (pattern: cfg(test) test_paths, persistence/mod.rs)",
          "under-threshold / threshold-crossed: successive append/append_line calls — the only mutation API",
          "rotated-3-deep: repeated append_line past a small max_bytes, or pre-written x.log.1/.2/.3 fixtures in the TempDir to exercise the rename chain"
        ],
        "budgets": [
          "rotation threshold 5 MiB = 5_242_880 bytes (DEFAULT_MAX_BYTES); unit tests may pass a smaller max_bytes",
          "3 rotated files kept (ROTATIONS_KEPT = 3): x.log.1, x.log.2, x.log.3",
          "per rotation: 1 remove + 3 renames + 1 reopen; byte count tracked in memory, zero stat() calls after open",
          "file 0600, directory 0700 (Unix)",
          "in-memory LogBuffer cap 10_000 lines unchanged (crates/process/src/log_buffer.rs)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "test rotating_writer_rotates_at_threshold: open with small max_bytes, append past it, assert x.log.1 exists, x.log holds only post-rotation lines, every file <= max_bytes",
        "test rotating_writer_keeps_at_most_three_rotations: force four rotations, assert exactly x.log, x.log.1, x.log.2, x.log.3 remain and the oldest was deleted",
        "test rotating_writer_creates_private_file_and_dir: assert directory mode 0o700 and file mode 0o600 (std::os::unix::fs::PermissionsExt)",
        "test apppaths_logs_dir_per_profile: AppPaths::for_profile_in for Test plus for_profile_with_env with a fake Env for production and development; assert logs dir is <state_dir>/logs with per-profile qualifiers v2ray-rs / v2ray-rs-dev / v2ray-rs-test",
        "implement crates/core/src/rotating_log.rs (RotatingFileWriter, open, append, append_line, DEFAULT_MAX_BYTES, ROTATIONS_KEPT) and export pub mod rotating_log from lib.rs",
        "add AppPaths::logs_dir() in crates/core/src/persistence/mod.rs next to latency_snapshot_path (lines 217-219)",
        "run make test-core; existing persistence/settings/subscriptions/tun_session tests stay green"
      ],
      "redTests": [],
      "redRun": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4",
      "verify": "cargo check -p v2ray-rs-core && make test-core && cargo clippy -p v2ray-rs-core --all-targets -- -D warnings"
    },
    {
      "id": "ui-audit",
      "taskIds": [
        "2.3"
      ],
      "prev": null,
      "sharedPkg": "v2ray-rs-ui",
      "parallel": true,
      "seam": "ui-app-logger",
      "shard": "ui-audit",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [],
      "coder": "rust-coder",
      "sites": [
        {
          "task": "2.3",
          "file": "crates/ui/src/subscriptions.rs",
          "symbol": "log-site audit",
          "anchor": "updated subscription {id}: +{} -{} ={} failed={}",
          "change": "replace subscription {id} with the display name resolved by id at subscriptions.rs:896, :981, :1018-1023; audit every log:: call site across crates against R3 (no URLs, UUIDs, passwords)"
        }
      ],
      "contract": {
        "states": [
          "uninstalled",
          "stderr-only",
          "file-and-stderr",
          "level-overridden"
        ],
        "transitions": [
          {
            "input": "startup with writable state dir (init_logging right after paths resolution)",
            "state": "file-and-stderr",
            "effect": "set",
            "evidence": "R1; app.rs:2340-2346 paths, app.rs:2415 first persistence load"
          },
          {
            "input": "startup with writer open failure",
            "state": "stderr-only",
            "effect": "forced",
            "evidence": "design risk line: logging never blocks or fails a connection"
          },
          {
            "input": "V2RAY_RS_LOG=<trace|debug|info|warn|error> at launch",
            "state": "level-overridden",
            "effect": "set",
            "evidence": "R1 env override; design D3"
          },
          {
            "input": "V2RAY_RS_LOG=<invalid value> at launch",
            "state": "level-overridden",
            "effect": "forced",
            "evidence": "design D3 default info; one stderr note"
          },
          {
            "input": "log::warn!/error!/info! at or above installed level",
            "state": "file-and-stderr",
            "effect": "set",
            "evidence": "R1 scenario warning reaches file"
          },
          {
            "input": "log::debug!/trace! below installed level (default info)",
            "state": "file-and-stderr",
            "effect": "no-op",
            "evidence": "task 2.1 unit test: debug does not land by default"
          },
          {
            "input": "launch under a different profile qualifier (dev vs production)",
            "state": "file-and-stderr",
            "effect": "set",
            "evidence": "R1 scenario profiles keep separate logs; persistence/mod.rs:98-102, profile.rs:23-31"
          }
        ],
        "forbidden": [
          "debug or trace records present in v2ray-rs.log at default level",
          "application records containing subscription URLs, node UUIDs or node passwords (R3, task 2.3 audit)",
          "log::set_boxed_logger called from a unit test (process-global; races sibling tests)",
          "logger installed after load_settings (app.rs:2415) — early persistence failures would be lost (R1)"
        ],
        "seeding": [
          "unit tests construct AppLogger::new(path_inside_tempdir, LevelFilter) directly and invoke log::Log::log(&logger, &record) — never the global installer",
          "resolve_level is pure: pass None, valid values in mixed case, and invalid values",
          "the installed path (task 2.2) is seeded only by a real dev-profile launch, not by unit tests"
        ],
        "budgets": [
          "default level info; V2RAY_RS_LOG accepted set exactly trace|debug|info|warn|error (case-insensitive); invalid -> info + 1 stderr line",
          "v2ray-rs.log bounded by seam core-rotating-writer-paths: 5 MiB threshold, 3 rotated files",
          "zero persistence records emitted before the logger is installed (install precedes load_settings, app.rs:2415)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "task 2.3 replacements: subscriptions.rs:896, :981, :1017-1023 switch {id} to the subscription display name resolved by id",
        "audit re-run: rg -n 'log::(info|warn|error|debug)!' crates reviewed against R3 (no subscription URLs, no UUIDs, no node credentials)"
      ],
      "redTests": [],
      "redRun": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4",
      "verify": "cargo check -p v2ray-rs-ui && make test-ui && cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings"
    },
    {
      "id": "process-log",
      "taskIds": [
        "3.1",
        "3.2"
      ],
      "prev": "core-writer",
      "sharedPkg": null,
      "parallel": true,
      "seam": "process-backend-log",
      "shard": "process",
      "pkgDirs": [
        "crates/process/src"
      ],
      "pkgs": [],
      "coder": "rust-coder",
      "sites": [
        {
          "task": "3.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::with_log_file",
          "anchor": "pub fn with_backend(mut self, backend: BackendType) -> Self {",
          "change": "add pub fn with_log_file(mut self, writer: Option<Arc<RotatingFileWriter>>) -> Self + log_writer field default None; reader tasks write append_line(\"stdout\"|\"stderr\", line) BEFORE buffer push; log_helper adds append_line(\"helper\", content)"
        },
        {
          "task": "3.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::launch",
          "anchor": "async fn launch(&mut self) -> Result<(), ProcessError> {",
          "change": "session record before try_spawn and on every respawn; exit records at graceful_stop / handle_unexpected_exit / wait-error path; gated cached version probe (writer attached only, CONFIG_CHECK_TIMEOUT bound)"
        }
      ],
      "contract": {
        "states": [
          "no-writer",
          "armed",
          "session-written",
          "lines-streaming",
          "exit-written"
        ],
        "transitions": [
          {
            "input": "with_log_file(None) or plain construction",
            "state": "no-writer",
            "effect": "no-op",
            "evidence": "manager.rs:97-121 constructor default; R2 dormant, behavior identical to today"
          },
          {
            "input": "with_log_file(Some(writer)) at construction",
            "state": "armed",
            "effect": "set",
            "evidence": "task 3.1; design D4 shared writer"
          },
          {
            "input": "launch() with writer attached (start or respawn)",
            "state": "session-written",
            "effect": "set",
            "evidence": "task 3.2; R2 each launch preceded by session record; manager.rs:336-368, respawn manager.rs:312-320"
          },
          {
            "input": "backend stdout/stderr line in a reader task",
            "state": "lines-streaming",
            "effect": "set",
            "evidence": "task 3.1; R2 lines not lost under load; file write precedes buffer push, manager.rs:446-483"
          },
          {
            "input": "route-helper output line via log_helper",
            "state": "lines-streaming",
            "effect": "set",
            "evidence": "R2 every route-helper line; manager.rs:593-608, HelperRun tun.rs:212-215"
          },
          {
            "input": "requested stop through graceful_stop",
            "state": "exit-written",
            "effect": "set",
            "evidence": "task 3.2 requested-stop test; manager.rs:461-481, SIGTERM then 5 s STOP_TIMEOUT then SIGKILL"
          },
          {
            "input": "unrequested exit while Running (handle_unexpected_exit after record_crash)",
            "state": "exit-written",
            "effect": "set",
            "evidence": "task 3.2 crash test; R2 crash reason readable; manager.rs:484-588, MAX_CRASHES 3 per CRASH_WINDOW 60 s manager.rs:19-27"
          },
          {
            "input": "child wait error (status unknowable)",
            "state": "exit-written",
            "effect": "forced",
            "evidence": "manager.rs:320-329; code=none"
          },
          {
            "input": "version probe timeout or unparsable output",
            "state": "session-written",
            "effect": "forced",
            "evidence": "design non-goal: probe never blocks start; CONFIG_CHECK_TIMEOUT 10 s, cached after first attempt"
          }
        ],
        "forbidden": [
          "an exit record in backend.log without a prior session record from the same manager",
          "lines missing from backend.log when the broadcast receiver lags or is never polled (20,000-line burst, task 3.1)",
          "version probe spawning when no writer is attached (existing stub tests would stall, manager.rs:714/:1018)",
          "exit record written before cleanup_after_exit drains the readers — last_output would be stale (manager.rs:610-625)",
          "writer io errors propagating into ProcessManager, or any await on the writer inside reader tasks (append is sync and infallible)"
        ],
        "seeding": [
          "no-writer: manager_for(&dir, script) exactly as today (manager.rs:683-688)",
          "armed: manager_for(...).with_log_file(Some(Arc::new(RotatingFileWriter::open(dir.path().join(\"backend.log\"), max).unwrap()))) — the only constructor path",
          "session-written / lines-streaming: mgr.start().await with a write_script stub (manager.rs:675-681); scripts that assert version add [ \"$1\" = version ] echo handling",
          "exit-written requested: mgr.start().await then mgr.stop().await; exit-written unrequested: script echo fatal >&2 then exit 3 with set_auto_restart(false) (pattern crash_error_includes_last_stderr_line, manager.rs:724-727) or kill -SEGV the live child pid"
        ],
        "budgets": [
          "20,000-line burst with no receiver polled: 0 lines lost (task 3.1)",
          "backend.log 5 MiB threshold, 3 rotated files (seam core-rotating-writer-paths)",
          "exactly 1 session line and 1 exit line per launch",
          "version probe: at most once per ProcessManager, only when a writer is attached, timeout 10 s (CONFIG_CHECK_TIMEOUT, manager.rs:19-27)",
          "crashes_in_window bounded by MAX_CRASHES = 3 per CRASH_WINDOW = 60 s (manager.rs:19-27)",
          "graceful stop: SIGTERM, 5 s STOP_TIMEOUT, then SIGKILL (manager.rs:19-27)",
          "reader drain before exit record: 500 ms LOG_DRAIN_TIMEOUT (manager.rs:610-625)",
          "last_output capped at REASON_MAX_CHARS = 200 chars (truncate_reason, manager.rs:636-643)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "test backend_log_captures_all_lines_under_load: stub backend printing 20,000 numbered stdout lines then sleeping; never call subscribe_logs; assert all 20,000 lines are in backend.log in order",
        "test session_record_precedes_spawn: start with writer attached, assert the first backend.log line contains session backend=... version=... node=... tun=... (the writer prepends the RFC3339 timestamp to every line, so assert by substring) and precedes every stdout/stderr line",
        "test exit_record_marks_requested_stop: start then stop, assert a line containing exit requested=true with code or signal field and crashes_in_window=0 (substring: writer timestamp prefix)",
        "test exit_record_marks_unrequested_crash: crash the stub (exit 3, auto_restart off, or kill -SEGV), assert (by substring) exit requested=false with code=3 or signal=11, crashes_in_window=1, and last_output carrying the final stderr line",
        "implement with_log_file + log_writer field; reader tasks call append_line before buffer push (manager.rs:446-483)",
        "implement write_exit_record with ExitStatusExt signal extraction and the three call sites (graceful_stop, handle_unexpected_exit after record_crash, wait-error path)",
        "implement the gated cached version probe (backend_version) and the session record in launch()",
        "log_helper writes append_line(\"helper\", content) per line (manager.rs:593-608)",
        "run make test-process; the whole existing process suite stays green"
      ],
      "redTests": [],
      "redRun": "timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4",
      "verify": "cargo check -p v2ray-rs-process && make test-process && cargo clippy -p v2ray-rs-process --all-targets -- -D warnings"
    },
    {
      "id": "ui-logger",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "prev": "core-writer",
      "sharedPkg": "v2ray-rs-ui",
      "parallel": true,
      "seam": "ui-app-logger",
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [],
      "coder": "rust-coder",
      "sites": [
        {
          "task": "2.1",
          "file": "crates/ui/src/logging.rs",
          "symbol": "AppLogger",
          "anchor": "pub mod i18n;",
          "change": "new file: log::Log impl writing <rfc3339> <LEVEL> <target> <message> to rotating v2ray-rs.log + stderr; resolve_level for V2RAY_RS_LOG (default info, invalid -> info + one stderr note); init_logging falls back to stderr-only on open failure; declared pub(crate) mod logging in crates/ui/src/lib.rs"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "try_run",
          "anchor": "fn try_run() -> Result<(), String> {",
          "change": "install init_logging immediately after AppPaths resolution and before the first persistence call (load_settings); emit one info startup record naming profile qualifier and version"
        }
      ],
      "contract": {
        "states": [
          "uninstalled",
          "stderr-only",
          "file-and-stderr",
          "level-overridden"
        ],
        "transitions": [
          {
            "input": "startup with writable state dir (init_logging right after paths resolution)",
            "state": "file-and-stderr",
            "effect": "set",
            "evidence": "R1; app.rs:2340-2346 paths, app.rs:2415 first persistence load"
          },
          {
            "input": "startup with writer open failure",
            "state": "stderr-only",
            "effect": "forced",
            "evidence": "design risk line: logging never blocks or fails a connection"
          },
          {
            "input": "V2RAY_RS_LOG=<trace|debug|info|warn|error> at launch",
            "state": "level-overridden",
            "effect": "set",
            "evidence": "R1 env override; design D3"
          },
          {
            "input": "V2RAY_RS_LOG=<invalid value> at launch",
            "state": "level-overridden",
            "effect": "forced",
            "evidence": "design D3 default info; one stderr note"
          },
          {
            "input": "log::warn!/error!/info! at or above installed level",
            "state": "file-and-stderr",
            "effect": "set",
            "evidence": "R1 scenario warning reaches file"
          },
          {
            "input": "log::debug!/trace! below installed level (default info)",
            "state": "file-and-stderr",
            "effect": "no-op",
            "evidence": "task 2.1 unit test: debug does not land by default"
          },
          {
            "input": "launch under a different profile qualifier (dev vs production)",
            "state": "file-and-stderr",
            "effect": "set",
            "evidence": "R1 scenario profiles keep separate logs; persistence/mod.rs:98-102, profile.rs:23-31"
          }
        ],
        "forbidden": [
          "debug or trace records present in v2ray-rs.log at default level",
          "application records containing subscription URLs, node UUIDs or node passwords (R3, task 2.3 audit)",
          "log::set_boxed_logger called from a unit test (process-global; races sibling tests)",
          "logger installed after load_settings (app.rs:2415) — early persistence failures would be lost (R1)"
        ],
        "seeding": [
          "unit tests construct AppLogger::new(path_inside_tempdir, LevelFilter) directly and invoke log::Log::log(&logger, &record) — never the global installer",
          "resolve_level is pure: pass None, valid values in mixed case, and invalid values",
          "the installed path (task 2.2) is seeded only by a real dev-profile launch, not by unit tests"
        ],
        "budgets": [
          "default level info; V2RAY_RS_LOG accepted set exactly trace|debug|info|warn|error (case-insensitive); invalid -> info + 1 stderr line",
          "v2ray-rs.log bounded by seam core-rotating-writer-paths: 5 MiB threshold, 3 rotated files",
          "zero persistence records emitted before the logger is installed (install precedes load_settings, app.rs:2415)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "test app_logger_writes_warn_and_filters_debug_by_default: AppLogger on a TempDir at LevelFilter::Info, Log::log a warn record and a debug record, assert the warn line (<rfc3339> WARN <target> <message> — exactly one timestamp, from the writer) is in the file and the debug line is absent",
        "test log_level_resolves_v2ray_rs_log: resolve_level(None) = Info, Some(\"debug\") = Debug, Some(\"WARN\") = Warn (case-insensitive), Some(\"bogus\") = Info",
        "implement crates/ui/src/logging.rs (AppLogger formats LEVEL target message and calls writer.append — the writer supplies the timestamp; resolve_level, init_logging) and register pub(crate) mod logging in crates/ui/src/lib.rs",
        "wire init_logging into try_run after app.rs:2346 and emit one info startup record naming the profile qualifier and version; add the dev-profile launch smoke (task 2.2): timeout 20 cargo run -p v2ray-rs-ui -- --profile development, then assert the startup record exists under the dev state logs dir (interactive: needs a display)"
      ],
      "redTests": [],
      "redRun": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4",
      "verify": "cargo check -p v2ray-rs-ui && make test-ui && cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings"
    },
    {
      "id": "ui-toasts",
      "taskIds": [
        "4.1",
        "4.2"
      ],
      "prev": "ui-logger",
      "sharedPkg": "v2ray-rs-ui",
      "parallel": false,
      "seam": "ui-failure-toasts",
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [],
      "coder": "rust-coder",
      "sites": [
        {
          "task": "4.1",
          "file": "crates/ui/src/geodata_service.rs",
          "symbol": "run_loop",
          "anchor": "async fn run_loop(mut config_rx: tokio::sync::watch::Receiver<GeodataRefreshConfig>) {",
          "change": "FailureStreak + pure refresh_outcome_toast; toast channel (UnboundedSender<String>) to an app.rs forwarder mapping to AppMsg::ShowToast; per-failure log::warn! stays"
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/failure_streak.rs",
          "symbol": "FailureStreak",
          "anchor": "pub mod i18n;",
          "change": "new file: pub(crate) struct FailureStreak { failed: bool }, record_failure -> bool (true only on clean-to-failing transition), record_success clears; declared pub(crate) mod failure_streak in lib.rs"
        },
        {
          "task": "4.2",
          "file": "crates/ui/src/subscriptions.rs",
          "symbol": "SubscriptionsCmdOutput::AutoUpdateDone",
          "anchor": "SubscriptionsCmdOutput::AutoUpdateDone(results) => {",
          "change": "auto_update_streaks HashMap<Uuid, FailureStreak> on the model; transition toasts naming the subscription via SubscriptionsOutput::Notice; record_success on Ok"
        }
      ],
      "contract": {
        "states": [
          "clean",
          "failing-first",
          "failing-repeat",
          "recovered"
        ],
        "transitions": [
          {
            "input": "geodata refresh or subscription auto-update failure from clean (startup or last success)",
            "state": "failing-first",
            "effect": "set",
            "evidence": "R4 scenario first failure toasts; design D5; geodata_service.rs:212-213, subscriptions.rs:1023"
          },
          {
            "input": "failure while already failing",
            "state": "failing-repeat",
            "effect": "no-op",
            "evidence": "R4 scenario repeated failure stays quiet but logged; log::warn stays at geodata_service.rs:212-213 / subscriptions.rs:1023"
          },
          {
            "input": "success while failing",
            "state": "recovered",
            "effect": "clear",
            "evidence": "R4 scenario recovery resets the streak; design D5 later success clears the flag"
          },
          {
            "input": "success while clean",
            "state": "clean",
            "effect": "no-op",
            "evidence": "design D5"
          }
        ],
        "forbidden": [
          "a toast emitted on a non-transition failure (second consecutive failure)",
          "a failure with neither a toast-transition nor a log::warn!/log::error! record (R4 requires logging every failure)",
          "toast text containing a subscription URL or node credential — subscription display name and error string only (R3)",
          "streak state cleared by anything other than a success of the same source"
        ],
        "seeding": [
          "pure: FailureStreak::new() then record_failure/record_success; no GTK objects in tests",
          "geodata path: refresh_outcome_toast(&mut streak, &Ok(())) / &Err(String) values",
          "subscriptions path: the helper over &mut HashMap<uuid::Uuid, FailureStreak> with (id, name, Result) — seeded by insertion only"
        ],
        "budgets": [
          "exactly 1 toast per failure streak (transition count = 1)",
          "scheduling unchanged: geodata loop period interval_secs, subscription tick >= 60 s (subscriptions.rs:1084)",
          "auto_update_streaks memory grows with the subscription count only"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "test geodata_failure_toasts_once_per_streak: drive refresh_outcome_toast — fail -> Some, fail -> None, fail -> None, success -> cleared, fail -> Some again",
        "test subscription_failure_toasts_once_per_streak: same shape through the subscriptions helper; assert the toast text names the subscription",
        "implement crates/ui/src/failure_streak.rs (FailureStreak, record_failure -> bool, record_success) and pub(crate) mod failure_streak in lib.rs",
        "geodata: extend GeodataRefreshService::spawn with the UnboundedSender<String> param, add refresh_outcome_toast, use it in run_loop keeping the per-failure log::warn!",
        "app.rs: create the channel at service spawn (app.rs:869) and add the forwarder task mapping messages to AppMsg::ShowToast",
        "subscriptions: auto_update_streaks field on the model init, transition toasts via SubscriptionsOutput::Notice in AutoUpdateDone (subscriptions.rs:992-1028), record_success on Ok",
        "run make test-ui and cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings"
      ],
      "redTests": [],
      "redRun": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4",
      "verify": "make test-ui && cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings"
    },
    {
      "id": "ui-connect",
      "taskIds": [
        "3.3"
      ],
      "prev": "process-log",
      "sharedPkg": "v2ray-rs-ui",
      "parallel": false,
      "seam": "process-backend-log",
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [],
      "coder": "rust-coder",
      "sites": [
        {
          "task": "3.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn",
          "anchor": "pub(super) fn spawn(request: ConnectionRequest, sender: relm4::Sender<AppMsg>) -> ConnectionHandle {",
          "change": "open one Arc<RotatingFileWriter> at paths.logs_dir().join(\"backend.log\") with DEFAULT_MAX_BYTES before the candidate loop; chain .with_log_file at the manager construction site; None + log::warn on open failure"
        }
      ],
      "contract": {
        "states": [
          "no-writer",
          "armed",
          "session-written",
          "lines-streaming",
          "exit-written"
        ],
        "transitions": [
          {
            "input": "with_log_file(None) or plain construction",
            "state": "no-writer",
            "effect": "no-op",
            "evidence": "manager.rs:97-121 constructor default; R2 dormant, behavior identical to today"
          },
          {
            "input": "with_log_file(Some(writer)) at construction",
            "state": "armed",
            "effect": "set",
            "evidence": "task 3.1; design D4 shared writer"
          },
          {
            "input": "launch() with writer attached (start or respawn)",
            "state": "session-written",
            "effect": "set",
            "evidence": "task 3.2; R2 each launch preceded by session record; manager.rs:336-368, respawn manager.rs:312-320"
          },
          {
            "input": "backend stdout/stderr line in a reader task",
            "state": "lines-streaming",
            "effect": "set",
            "evidence": "task 3.1; R2 lines not lost under load; file write precedes buffer push, manager.rs:446-483"
          },
          {
            "input": "route-helper output line via log_helper",
            "state": "lines-streaming",
            "effect": "set",
            "evidence": "R2 every route-helper line; manager.rs:593-608, HelperRun tun.rs:212-215"
          },
          {
            "input": "requested stop through graceful_stop",
            "state": "exit-written",
            "effect": "set",
            "evidence": "task 3.2 requested-stop test; manager.rs:461-481, SIGTERM then 5 s STOP_TIMEOUT then SIGKILL"
          },
          {
            "input": "unrequested exit while Running (handle_unexpected_exit after record_crash)",
            "state": "exit-written",
            "effect": "set",
            "evidence": "task 3.2 crash test; R2 crash reason readable; manager.rs:484-588, MAX_CRASHES 3 per CRASH_WINDOW 60 s manager.rs:19-27"
          },
          {
            "input": "child wait error (status unknowable)",
            "state": "exit-written",
            "effect": "forced",
            "evidence": "manager.rs:320-329; code=none"
          },
          {
            "input": "version probe timeout or unparsable output",
            "state": "session-written",
            "effect": "forced",
            "evidence": "design non-goal: probe never blocks start; CONFIG_CHECK_TIMEOUT 10 s, cached after first attempt"
          }
        ],
        "forbidden": [
          "an exit record in backend.log without a prior session record from the same manager",
          "lines missing from backend.log when the broadcast receiver lags or is never polled (20,000-line burst, task 3.1)",
          "version probe spawning when no writer is attached (existing stub tests would stall, manager.rs:714/:1018)",
          "exit record written before cleanup_after_exit drains the readers — last_output would be stale (manager.rs:610-625)",
          "writer io errors propagating into ProcessManager, or any await on the writer inside reader tasks (append is sync and infallible)"
        ],
        "seeding": [
          "no-writer: manager_for(&dir, script) exactly as today (manager.rs:683-688)",
          "armed: manager_for(...).with_log_file(Some(Arc::new(RotatingFileWriter::open(dir.path().join(\"backend.log\"), max).unwrap()))) — the only constructor path",
          "session-written / lines-streaming: mgr.start().await with a write_script stub (manager.rs:675-681); scripts that assert version add [ \"$1\" = version ] echo handling",
          "exit-written requested: mgr.start().await then mgr.stop().await; exit-written unrequested: script echo fatal >&2 then exit 3 with set_auto_restart(false) (pattern crash_error_includes_last_stderr_line, manager.rs:724-727) or kill -SEGV the live child pid"
        ],
        "budgets": [
          "20,000-line burst with no receiver polled: 0 lines lost (task 3.1)",
          "backend.log 5 MiB threshold, 3 rotated files (seam core-rotating-writer-paths)",
          "exactly 1 session line and 1 exit line per launch",
          "version probe: at most once per ProcessManager, only when a writer is attached, timeout 10 s (CONFIG_CHECK_TIMEOUT, manager.rs:19-27)",
          "crashes_in_window bounded by MAX_CRASHES = 3 per CRASH_WINDOW = 60 s (manager.rs:19-27)",
          "graceful stop: SIGTERM, 5 s STOP_TIMEOUT, then SIGKILL (manager.rs:19-27)",
          "reader drain before exit record: 500 ms LOG_DRAIN_TIMEOUT (manager.rs:610-625)",
          "last_output capped at REASON_MAX_CHARS = 200 chars (truncate_reason, manager.rs:636-643)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "task 3.3: connection.rs opens the shared Arc<RotatingFileWriter> at paths.logs_dir().join(\"backend.log\") before the candidate loop and chains .with_log_file at connection.rs:180-187; None + log::warn on open failure"
      ],
      "redTests": [],
      "redRun": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4",
      "verify": "cargo check -p v2ray-rs-ui && make test-ui && cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings"
    },
    {
      "id": "docs-floor",
      "taskIds": [
        "5.1",
        "5.3"
      ],
      "prev": "ui-connect",
      "sharedPkg": null,
      "parallel": true,
      "seam": "verify-docs-live",
      "shard": "docs",
      "pkgDirs": [],
      "pkgs": [],
      "coder": "coder",
      "sites": [
        {
          "task": "5.1",
          "file": "Cargo.toml",
          "symbol": "workspace",
          "anchor": "[workspace]",
          "change": "verification only: make test TEST_TIMEOUT=10m green across the workspace"
        },
        {
          "task": "5.3",
          "file": "CHANGELOG.md",
          "symbol": "[Unreleased]",
          "anchor": "## [Unreleased]",
          "change": "Added entries: rotating application/backend logs, session/exit records, V2RAY_RS_LOG, once-per-streak failure toasts; docs/ARCHITECTURE.md logging section near ## On-disk layout with exact paths, rotation budgets and the env var"
        }
      ],
      "contract": {
        "states": [
          "undocumented",
          "changelog-entry",
          "docs-section",
          "live-verified"
        ],
        "transitions": [
          {
            "input": "make test TEST_TIMEOUT=10m green",
            "state": "live-verified",
            "effect": "set",
            "evidence": "task 5.1"
          },
          {
            "input": "SEGV reproduction observed in relaunched backend.log",
            "state": "live-verified",
            "effect": "set",
            "evidence": "task 5.2; R2 scenario"
          },
          {
            "input": "CHANGELOG.md [Unreleased] Added entries written",
            "state": "changelog-entry",
            "effect": "set",
            "evidence": "task 5.3"
          },
          {
            "input": "docs/ARCHITECTURE.md logging section written",
            "state": "docs-section",
            "effect": "set",
            "evidence": "task 5.3"
          },
          {
            "input": "any workspace suite red",
            "state": "undocumented",
            "effect": "clear",
            "evidence": "task 5.1 gate"
          }
        ],
        "forbidden": [
          "docs naming paths or env vars with spelling that differs from the implementation (V2RAY_RS_LOG, v2ray-rs.log, backend.log, <state_dir>/logs)",
          "5.2 claimed from a requested stop — only an unrequested signal exit proves the record",
          "5.1 run with fewer than --test-threads=4 or without the 10 m timeout wrapper"
        ],
        "seeding": [
          "live states need the dev profile launched with a display and a real backend installed; scripted parts: kill -SEGV the pid from the dev runtime backend.pid, then grep backend.log for the exit record"
        ],
        "budgets": [
          "make test TEST_TIMEOUT=10m = timeout 10m cargo test --workspace --all-targets -- --test-threads=4",
          "make lint = cargo fmt -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "5.1: run make test TEST_TIMEOUT=10m; fix fallout only in crates touched by this change",
        "5.3: CHANGELOG.md [Unreleased] Added entries for rotating application/backend logs, session/exit records, V2RAY_RS_LOG, once-per-streak failure toasts; docs/ARCHITECTURE.md section with exact paths and budgets",
        "verify docs: rg -n 'V2RAY_RS_LOG|logs/v2ray-rs.log|logs/backend.log' CHANGELOG.md docs/ARCHITECTURE.md returns hits in both files"
      ],
      "redTests": [],
      "redRun": "make test TEST_TIMEOUT=10m",
      "verify": "make test TEST_TIMEOUT=10m && grep -rn V2RAY_RS_LOG CHANGELOG.md docs/ARCHITECTURE.md"
    },
    {
      "id": "live-segv",
      "taskIds": [
        "5.2"
      ],
      "prev": "docs-floor",
      "sharedPkg": null,
      "parallel": true,
      "seam": "verify-docs-live",
      "shard": "live",
      "pkgDirs": [],
      "pkgs": [],
      "coder": "coder",
      "sites": [
        {
          "task": "5.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "handle_unexpected_exit",
          "anchor": "async fn handle_unexpected_exit(&mut self, exit_code: Option<i32>) {",
          "change": "verification only: guided-live — launch dev profile, connect, kill -SEGV the backend pid from the dev runtime backend.pid, quit, relaunch, assert backend.log holds the last lines and exit requested=false signal=11"
        }
      ],
      "contract": {
        "states": [
          "undocumented",
          "changelog-entry",
          "docs-section",
          "live-verified"
        ],
        "transitions": [
          {
            "input": "make test TEST_TIMEOUT=10m green",
            "state": "live-verified",
            "effect": "set",
            "evidence": "task 5.1"
          },
          {
            "input": "SEGV reproduction observed in relaunched backend.log",
            "state": "live-verified",
            "effect": "set",
            "evidence": "task 5.2; R2 scenario"
          },
          {
            "input": "CHANGELOG.md [Unreleased] Added entries written",
            "state": "changelog-entry",
            "effect": "set",
            "evidence": "task 5.3"
          },
          {
            "input": "docs/ARCHITECTURE.md logging section written",
            "state": "docs-section",
            "effect": "set",
            "evidence": "task 5.3"
          },
          {
            "input": "any workspace suite red",
            "state": "undocumented",
            "effect": "clear",
            "evidence": "task 5.1 gate"
          }
        ],
        "forbidden": [
          "docs naming paths or env vars with spelling that differs from the implementation (V2RAY_RS_LOG, v2ray-rs.log, backend.log, <state_dir>/logs)",
          "5.2 claimed from a requested stop — only an unrequested signal exit proves the record",
          "5.1 run with fewer than --test-threads=4 or without the 10 m timeout wrapper"
        ],
        "seeding": [
          "live states need the dev profile launched with a display and a real backend installed; scripted parts: kill -SEGV the pid from the dev runtime backend.pid, then grep backend.log for the exit record"
        ],
        "budgets": [
          "make test TEST_TIMEOUT=10m = timeout 10m cargo test --workspace --all-targets -- --test-threads=4",
          "make lint = cargo fmt -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "guided-live 5.2: run the scripted assertions a headless session allows; interactive GUI steps (connect, quit, relaunch) are handed to the user in the run report with the exact command sequence; the unit test exit_record_marks_unrequested_crash covers the signal=11 record path"
      ],
      "redTests": [],
      "redRun": "timeout 5m cargo test -p v2ray-rs-process exit_record_marks_unrequested_crash -- --test-threads=4",
      "verify": "timeout 5m cargo test -p v2ray-rs-process exit_record_marks_unrequested_crash -- --test-threads=4"
    }
  ],
  "seams": [
    {
      "id": "core-rotating-writer-paths",
      "tasks": [
        "1.1",
        "1.2"
      ],
      "summary": "NO-RED-WAIVER: Rust seam, no test-writer agent; tests are written by rust-coder as its first codeTasks. NO-TESTER-WAIVER: redRun is cargo test, not a go test command; the chunk closes by waiver with tests run in-slice. New module crates/core/src/rotating_log.rs with pub struct RotatingFileWriter (Send + Sync via interior Mutex, shared as Arc). open(path: &Path, max_bytes: u64) -> io::Result<Self> creates the parent directory 0700 through the existing pub(crate) create_dir_with_permissions (crates/core/src/persistence/mod.rs) and the file 0600 (OpenOptions append + mode(0o600) at creation), seeds its byte count from the existing file length once at open, then self-counts (design D2: no per-write stat). append(content: &str) writes one line <rfc3339> <content>; append_line(stream: &str, content: &str) writes <rfc3339> <stream> <content>; timestamp = chrono::Utc::now().to_rfc3339() (chrono already in crates/core/Cargo.toml). Before a write, if bytes + line + 1 > max_bytes: rotate — remove old x.log.3, rename x.log.2 -> x.log.3, x.log.1 -> x.log.2, x.log -> x.log.1, reopen fresh at byte 0; the crossing line starts the new file so every file stays <= max_bytes. First write io error is reported once to stderr (eprintln), later errors suppressed; append never returns Err, never panics, never blocks caller logic. Consts pub DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024 and pub ROTATIONS_KEPT: u32 = 3; tests pass smaller max_bytes. Task 1.2: AppPaths::logs_dir(&self) -> PathBuf = self.state_dir.join(\"logs\"), placed beside latency_snapshot_path (crates/core/src/persistence/mod.rs:217-219); state_dir resolves per profile at persistence/mod.rs:98-102 with qualifiers v2ray-rs / v2ray-rs-dev / v2ray-rs-test (crates/core/src/profile.rs:23-31), so profiles keep separate logs (R1 scenario). lib.rs gains pub mod rotating_log. No new dependencies. Existing tests that must keep passing: the whole core persistence suite (make test-core).",
      "contract": {
        "states": [
          "fresh-open",
          "under-threshold",
          "threshold-crossed",
          "rotated-3-deep"
        ],
        "transitions": [
          {
            "input": "RotatingFileWriter::open(path, max_bytes) on new or existing file",
            "state": "fresh-open",
            "effect": "set",
            "evidence": "task 1.1; R3 permissions scenario; state_dir base persistence/mod.rs:98-102"
          },
          {
            "input": "append/append_line while bytes + line + 1 <= max_bytes",
            "state": "under-threshold",
            "effect": "set",
            "evidence": "task 1.1 byte-counted threshold; design D2"
          },
          {
            "input": "append/append_line while bytes + line + 1 > max_bytes",
            "state": "threshold-crossed",
            "effect": "forced",
            "evidence": "task 1.1; design D2 rotate-then-reopen; R3 rotation scenario"
          },
          {
            "input": "rotation when x.log.1 and x.log.2 already exist",
            "state": "rotated-3-deep",
            "effect": "forced",
            "evidence": "task 1.1 three rotations, deletion of oldest; R3 at most three rotated files"
          },
          {
            "input": "write io error (full disk, removed dir)",
            "state": "under-threshold",
            "effect": "no-op",
            "evidence": "design risk line: write failures ignored after the first, reported to stderr"
          }
        ],
        "forbidden": [
          "any log file larger than max_bytes plus one line length",
          "a fourth rotated file (x.log.4) or an x.log.3 surviving the next rotation instead of being deleted",
          "log directory mode wider than 0o700 or file mode wider than 0o600 at creation",
          "append returning Err, panicking, or blocking on a retry loop"
        ],
        "seeding": [
          "fresh-open: only via RotatingFileWriter::open on a path inside a tempfile::TempDir (pattern: cfg(test) test_paths, persistence/mod.rs)",
          "under-threshold / threshold-crossed: successive append/append_line calls — the only mutation API",
          "rotated-3-deep: repeated append_line past a small max_bytes, or pre-written x.log.1/.2/.3 fixtures in the TempDir to exercise the rename chain"
        ],
        "budgets": [
          "rotation threshold 5 MiB = 5_242_880 bytes (DEFAULT_MAX_BYTES); unit tests may pass a smaller max_bytes",
          "3 rotated files kept (ROTATIONS_KEPT = 3): x.log.1, x.log.2, x.log.3",
          "per rotation: 1 remove + 3 renames + 1 reopen; byte count tracked in memory, zero stat() calls after open",
          "file 0600, directory 0700 (Unix)",
          "in-memory LogBuffer cap 10_000 lines unchanged (crates/process/src/log_buffer.rs)"
        ]
      },
      "codeTasks": [
        "test rotating_writer_rotates_at_threshold: open with small max_bytes, append past it, assert x.log.1 exists, x.log holds only post-rotation lines, every file <= max_bytes",
        "test rotating_writer_keeps_at_most_three_rotations: force four rotations, assert exactly x.log, x.log.1, x.log.2, x.log.3 remain and the oldest was deleted",
        "test rotating_writer_creates_private_file_and_dir: assert directory mode 0o700 and file mode 0o600 (std::os::unix::fs::PermissionsExt)",
        "test apppaths_logs_dir_per_profile: AppPaths::for_profile_in for Test plus for_profile_with_env with a fake Env for production and development; assert logs dir is <state_dir>/logs with per-profile qualifiers v2ray-rs / v2ray-rs-dev / v2ray-rs-test",
        "implement crates/core/src/rotating_log.rs (RotatingFileWriter, open, append, append_line, DEFAULT_MAX_BYTES, ROTATIONS_KEPT) and export pub mod rotating_log from lib.rs",
        "add AppPaths::logs_dir() in crates/core/src/persistence/mod.rs next to latency_snapshot_path (lines 217-219)",
        "run make test-core; existing persistence/settings/subscriptions/tun_session tests stay green"
      ]
    },
    {
      "id": "ui-app-logger",
      "tasks": [
        "2.1",
        "2.2",
        "2.3"
      ],
      "summary": "NO-RED-WAIVER: Rust seam, no test-writer agent; tests are written by rust-coder as its first codeTasks. NO-TESTER-WAIVER: redRun is cargo test, not a go test command; the chunk closes by waiver with tests run in-slice. New crates/ui/src/logging.rs: pub(crate) struct AppLogger { writer: Option<RotatingFileWriter> } implementing log::Log; each record line in the file is <rfc3339> <UPPERCASE level> <target> <message>: AppLogger formats only <UPPERCASE level> <target> <message> and delegates the single timestamp to RotatingFileWriter::append (never two timestamps on a line); stderr gets the same record with its timestamp. pub(crate) fn resolve_level(value: Option<&str>) -> log::LevelFilter parses V2RAY_RS_LOG, accepted set exactly {trace, debug, info, warn, error} case-insensitive, default info, invalid value falls back to info with one stderr note. pub(crate) fn init_logging(paths: &AppPaths) opens RotatingFileWriter at paths.logs_dir().join(\"v2ray-rs.log\") with DEFAULT_MAX_BYTES, installs via log::set_boxed_logger + log::set_max_level; on writer open failure it installs a stderr-only logger (writer = None) so startup never fails on logging. Task 2.2 install point: first act of try_run() immediately after AppPaths::with_overrides (crates/ui/src/app.rs:2340-2346) and before the first persistence call load_settings (app.rs:2415); crates/ui/src/main.rs is a three-line shim calling v2ray_rs_ui::run() and paths depend on CLI parsing (--profile/--state-dir), so first-in-main is pinned as first-in-startup-path-once-paths-are-known, then one info startup record naming profile qualifier and app version. Task 2.3 audit result (verified by grep over all 23+ log:: call sites): three sites print subscription UUIDs and must switch to the subscription display name — crates/ui/src/subscriptions.rs:896 (updated subscription {id}), :981 (failed to update subscription {id}), :1017-1023 (auto-updated {id} / auto-update {id} failed). Every other site prints paths, geodata tags, hostnames or error strings — reviewed clean against R3; node hostnames at crates/ui/src/connection.rs:321 stay (not a URL or credential; same trust boundary as backend output, design D6). Existing tests that must keep passing: make test-ui (app.rs reconnect tests, subscriptions.rs merge tests).",
      "contract": {
        "states": [
          "uninstalled",
          "stderr-only",
          "file-and-stderr",
          "level-overridden"
        ],
        "transitions": [
          {
            "input": "startup with writable state dir (init_logging right after paths resolution)",
            "state": "file-and-stderr",
            "effect": "set",
            "evidence": "R1; app.rs:2340-2346 paths, app.rs:2415 first persistence load"
          },
          {
            "input": "startup with writer open failure",
            "state": "stderr-only",
            "effect": "forced",
            "evidence": "design risk line: logging never blocks or fails a connection"
          },
          {
            "input": "V2RAY_RS_LOG=<trace|debug|info|warn|error> at launch",
            "state": "level-overridden",
            "effect": "set",
            "evidence": "R1 env override; design D3"
          },
          {
            "input": "V2RAY_RS_LOG=<invalid value> at launch",
            "state": "level-overridden",
            "effect": "forced",
            "evidence": "design D3 default info; one stderr note"
          },
          {
            "input": "log::warn!/error!/info! at or above installed level",
            "state": "file-and-stderr",
            "effect": "set",
            "evidence": "R1 scenario warning reaches file"
          },
          {
            "input": "log::debug!/trace! below installed level (default info)",
            "state": "file-and-stderr",
            "effect": "no-op",
            "evidence": "task 2.1 unit test: debug does not land by default"
          },
          {
            "input": "launch under a different profile qualifier (dev vs production)",
            "state": "file-and-stderr",
            "effect": "set",
            "evidence": "R1 scenario profiles keep separate logs; persistence/mod.rs:98-102, profile.rs:23-31"
          }
        ],
        "forbidden": [
          "debug or trace records present in v2ray-rs.log at default level",
          "application records containing subscription URLs, node UUIDs or node passwords (R3, task 2.3 audit)",
          "log::set_boxed_logger called from a unit test (process-global; races sibling tests)",
          "logger installed after load_settings (app.rs:2415) — early persistence failures would be lost (R1)"
        ],
        "seeding": [
          "unit tests construct AppLogger::new(path_inside_tempdir, LevelFilter) directly and invoke log::Log::log(&logger, &record) — never the global installer",
          "resolve_level is pure: pass None, valid values in mixed case, and invalid values",
          "the installed path (task 2.2) is seeded only by a real dev-profile launch, not by unit tests"
        ],
        "budgets": [
          "default level info; V2RAY_RS_LOG accepted set exactly trace|debug|info|warn|error (case-insensitive); invalid -> info + 1 stderr line",
          "v2ray-rs.log bounded by seam core-rotating-writer-paths: 5 MiB threshold, 3 rotated files",
          "zero persistence records emitted before the logger is installed (install precedes load_settings, app.rs:2415)"
        ]
      },
      "codeTasks": [
        "test app_logger_writes_warn_and_filters_debug_by_default: AppLogger on a TempDir at LevelFilter::Info, Log::log a warn record and a debug record, assert the warn line (<rfc3339> WARN <target> <message>) is in the file and the debug line is absent",
        "test log_level_resolves_v2ray_rs_log: resolve_level(None) = Info, Some(\"debug\") = Debug, Some(\"WARN\") = Warn (case-insensitive), Some(\"bogus\") = Info",
        "implement crates/ui/src/logging.rs (AppLogger, resolve_level, init_logging) and register pub(crate) mod logging in crates/ui/src/lib.rs",
        "wire init_logging into try_run after app.rs:2346 and emit one info startup record naming the profile qualifier and version; add the dev-profile launch smoke (task 2.2): timeout 20 cargo run -p v2ray-rs-ui -- --profile development, then assert the startup record exists under the dev state logs dir (interactive: needs a display)",
        "task 2.3 replacements: subscriptions.rs:896, :981, :1017-1023 switch {id} to the subscription display name resolved by id",
        "audit re-run: rg -n 'log::(info|warn|error|debug)!' crates reviewed against R3 (no subscription URLs, no UUIDs, no node credentials)",
        "run make test-ui and cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings"
      ]
    },
    {
      "id": "process-backend-log",
      "tasks": [
        "3.1",
        "3.2",
        "3.3"
      ],
      "summary": "NO-RED-WAIVER: Rust seam, no test-writer agent; tests are written by rust-coder as its first codeTasks. NO-TESTER-WAIVER: redRun is cargo test, not a go test command; the chunk closes by waiver with tests run in-slice. crates/process/src/manager.rs gains pub fn with_log_file(mut self, writer: Option<Arc<RotatingFileWriter>>) -> Self and field log_writer: Option<Arc<RotatingFileWriter>> (default None in new(), manager.rs:97-121); with no writer attached behavior is byte-identical to today. capture_output (manager.rs:446-483) clones the Option<Arc> into both reader tasks; each line is written to backend.log FIRST via append_line(\"stdout\"|\"stderr\", &line), then broadcast-send and buffer-push in the same iteration — lossless even with a lagging or absent receiver (the UI forwarder drops on Lagged today, crates/ui/src/connection.rs:236-243). Session record, one line per launch written in launch() before try_spawn (manager.rs:336-368), also on every respawn (manager.rs:312-320): append(\"session backend=<xray|v2ray|sing-box> version=<x.y.z|unknown> node=<label|none> tun=<on|off>\"). version comes from a probe of <binary> version gated on a writer being attached, run at most once per ProcessManager and cached in a new backend_version field, bounded by CONFIG_CHECK_TIMEOUT 10s (manager.rs:19-27), parsed with the existing parse_semver_triple, else unknown — the gate keeps existing stub tests (scripts like exec sleep 30 with no version arg handling, manager.rs:714 and :1018) from stalling. node label = current_connection node_name or none (set before launch, manager.rs:182-184); tun = self.tun.is_some(). Exit record via new helper write_exit_record(requested: bool, status: Option<&ExitStatus>): append(\"exit requested=<true|false> <code=N | signal=N | code=none> crashes_in_window=<K> last_output=<quoted line | none>\") — signal via std::os::unix::process::ExitStatusExt. Call sites: graceful_stop after the wait completes, requested=true, K = crash_times.len() (manager.rs:461-481); handle_unexpected_exit after record_crash, requested=false, K = crash_times.len() including the current crash (manager.rs:484-588); the wait-error path of wait_and_handle_exit, requested=false, code=none (manager.rs:320-329). last_output reuses last_output_line (manager.rs:627-641) truncated to 200 chars; exit records are written after cleanup_after_exit drained the readers (LOG_DRAIN_TIMEOUT 500 ms, manager.rs:610-625) so the last line is accurate. Route-helper output: log_helper (manager.rs:593-608) additionally append_line(\"helper\", content) for every HelperRun.output line (tun.rs:212-215). Task 3.3: crates/ui/src/connection.rs opens one Arc<RotatingFileWriter> at paths.logs_dir().join(\"backend.log\") with DEFAULT_MAX_BYTES before the candidate loop and chains .with_log_file at the manager construction site (connection.rs:180-187); on open failure it passes None and log::warn (connection still works). Note: the ProcessManager Drop path SIGKILLs a live child without an exit record (manager.rs:649-664) — accepted gap, UI paths always stop() through shutdown. Existing tests that must keep passing: the whole process suite (make test-process), including config_check_failure_prevents_spawn, crash_error_includes_last_stderr_line, respawn_budget_exhaustion_errors, stop_from_running_without_child_reaches_stopped.",
      "contract": {
        "states": [
          "no-writer",
          "armed",
          "session-written",
          "lines-streaming",
          "exit-written"
        ],
        "transitions": [
          {
            "input": "with_log_file(None) or plain construction",
            "state": "no-writer",
            "effect": "no-op",
            "evidence": "manager.rs:97-121 constructor default; R2 dormant, behavior identical to today"
          },
          {
            "input": "with_log_file(Some(writer)) at construction",
            "state": "armed",
            "effect": "set",
            "evidence": "task 3.1; design D4 shared writer"
          },
          {
            "input": "launch() with writer attached (start or respawn)",
            "state": "session-written",
            "effect": "set",
            "evidence": "task 3.2; R2 each launch preceded by session record; manager.rs:336-368, respawn manager.rs:312-320"
          },
          {
            "input": "backend stdout/stderr line in a reader task",
            "state": "lines-streaming",
            "effect": "set",
            "evidence": "task 3.1; R2 lines not lost under load; file write precedes buffer push, manager.rs:446-483"
          },
          {
            "input": "route-helper output line via log_helper",
            "state": "lines-streaming",
            "effect": "set",
            "evidence": "R2 every route-helper line; manager.rs:593-608, HelperRun tun.rs:212-215"
          },
          {
            "input": "requested stop through graceful_stop",
            "state": "exit-written",
            "effect": "set",
            "evidence": "task 3.2 requested-stop test; manager.rs:461-481, SIGTERM then 5 s STOP_TIMEOUT then SIGKILL"
          },
          {
            "input": "unrequested exit while Running (handle_unexpected_exit after record_crash)",
            "state": "exit-written",
            "effect": "set",
            "evidence": "task 3.2 crash test; R2 crash reason readable; manager.rs:484-588, MAX_CRASHES 3 per CRASH_WINDOW 60 s manager.rs:19-27"
          },
          {
            "input": "child wait error (status unknowable)",
            "state": "exit-written",
            "effect": "forced",
            "evidence": "manager.rs:320-329; code=none"
          },
          {
            "input": "version probe timeout or unparsable output",
            "state": "session-written",
            "effect": "forced",
            "evidence": "design non-goal: probe never blocks start; CONFIG_CHECK_TIMEOUT 10 s, cached after first attempt"
          }
        ],
        "forbidden": [
          "an exit record in backend.log without a prior session record from the same manager",
          "lines missing from backend.log when the broadcast receiver lags or is never polled (20,000-line burst, task 3.1)",
          "version probe spawning when no writer is attached (existing stub tests would stall, manager.rs:714/:1018)",
          "exit record written before cleanup_after_exit drains the readers — last_output would be stale (manager.rs:610-625)",
          "writer io errors propagating into ProcessManager, or any await on the writer inside reader tasks (append is sync and infallible)"
        ],
        "seeding": [
          "no-writer: manager_for(&dir, script) exactly as today (manager.rs:683-688)",
          "armed: manager_for(...).with_log_file(Some(Arc::new(RotatingFileWriter::open(dir.path().join(\"backend.log\"), max).unwrap()))) — the only constructor path",
          "session-written / lines-streaming: mgr.start().await with a write_script stub (manager.rs:675-681); scripts that assert version add [ \"$1\" = version ] echo handling",
          "exit-written requested: mgr.start().await then mgr.stop().await; exit-written unrequested: script echo fatal >&2 then exit 3 with set_auto_restart(false) (pattern crash_error_includes_last_stderr_line, manager.rs:724-727) or kill -SEGV the live child pid"
        ],
        "budgets": [
          "20,000-line burst with no receiver polled: 0 lines lost (task 3.1)",
          "backend.log 5 MiB threshold, 3 rotated files (seam core-rotating-writer-paths)",
          "exactly 1 session line and 1 exit line per launch",
          "version probe: at most once per ProcessManager, only when a writer is attached, timeout 10 s (CONFIG_CHECK_TIMEOUT, manager.rs:19-27)",
          "crashes_in_window bounded by MAX_CRASHES = 3 per CRASH_WINDOW = 60 s (manager.rs:19-27)",
          "graceful stop: SIGTERM, 5 s STOP_TIMEOUT, then SIGKILL (manager.rs:19-27)",
          "reader drain before exit record: 500 ms LOG_DRAIN_TIMEOUT (manager.rs:610-625)",
          "last_output capped at REASON_MAX_CHARS = 200 chars (truncate_reason, manager.rs:636-643)"
        ]
      },
      "codeTasks": [
        "test backend_log_captures_all_lines_under_load: stub backend printing 20,000 numbered stdout lines then sleeping; never call subscribe_logs; assert all 20,000 lines are in backend.log in order",
        "test session_record_precedes_spawn: start with writer attached, assert the first backend.log line matches session backend=... version=... node=... tun=... and precedes every stdout/stderr line",
        "test exit_record_marks_requested_stop: start then stop, assert an exit requested=true line with code or signal field and crashes_in_window=0",
        "test exit_record_marks_unrequested_crash: crash the stub (exit 3, auto_restart off, or kill -SEGV), assert exit requested=false with code=3 or signal=11, crashes_in_window=1, and last_output carrying the final stderr line",
        "implement with_log_file + log_writer field; reader tasks call append_line before buffer push (manager.rs:446-483)",
        "implement write_exit_record with ExitStatusExt signal extraction and the three call sites (graceful_stop, handle_unexpected_exit after record_crash, wait-error path)",
        "implement the gated cached version probe (backend_version) and the session record in launch()",
        "log_helper writes append_line(\"helper\", content) per line (manager.rs:593-608)",
        "task 3.3: connection.rs opens the shared Arc<RotatingFileWriter> at paths.logs_dir().join(\"backend.log\") before the candidate loop and chains .with_log_file at connection.rs:180-187; None + log::warn on open failure",
        "run make test-process; the whole existing process suite stays green"
      ]
    },
    {
      "id": "ui-failure-toasts",
      "tasks": [
        "4.1",
        "4.2"
      ],
      "summary": "NO-RED-WAIVER: Rust seam, no test-writer agent; tests are written by rust-coder as its first codeTasks. NO-TESTER-WAIVER: redRun is cargo test, not a go test command; the chunk closes by waiver with tests run in-slice. New tiny type crates/ui/src/failure_streak.rs: pub(crate) struct FailureStreak { failed: bool } with record_failure(&mut self) -> bool (true only on the clean-to-failing transition) and record_success(&mut self) (clears). Geodata (4.1): GeodataRefreshService::spawn gains a toast channel — spawn(initial: GeodataRefreshConfig, toast_tx: tokio::sync::mpsc::UnboundedSender<String>) -> Self; run_loop (crates/ui/src/geodata_service.rs:206-227) keeps a FailureStreak and a pure helper refresh_outcome_toast(streak: &mut FailureStreak, result: &Result<(), String>) -> Option<String> returning Some(\"Geodata update failed: {err}\") only on transition; the existing log::warn! at geodata_service.rs:212-213 stays for EVERY failure (R4 log-every-failure); app.rs spawns a forwarder mapping channel messages to AppMsg::ShowToast following the tokio-forwarder pattern of connection.rs:206-215. Subscriptions (4.2): model field auto_update_streaks: std::collections::HashMap<uuid::Uuid, FailureStreak>; in SubscriptionsCmdOutput::AutoUpdateDone (crates/ui/src/subscriptions.rs:992-1028) each (id, result) drives a pure helper returning Some(\"Auto-update failed for {name}: {err}\") on transition, with the name resolved from self.subscriptions by id; toasts go out via sender.output(SubscriptionsOutput::Notice(...)) which already forwards to AppMsg::ShowToast (app.rs:830, handler app.rs:1025-1027); successes call record_success. Startup state is clean, so the first failure after launch toasts (design D5). Existing tests that must keep passing: make test-ui.",
      "contract": {
        "states": [
          "clean",
          "failing-first",
          "failing-repeat",
          "recovered"
        ],
        "transitions": [
          {
            "input": "geodata refresh or subscription auto-update failure from clean (startup or last success)",
            "state": "failing-first",
            "effect": "set",
            "evidence": "R4 scenario first failure toasts; design D5; geodata_service.rs:212-213, subscriptions.rs:1023"
          },
          {
            "input": "failure while already failing",
            "state": "failing-repeat",
            "effect": "no-op",
            "evidence": "R4 scenario repeated failure stays quiet but logged; log::warn stays at geodata_service.rs:212-213 / subscriptions.rs:1023"
          },
          {
            "input": "success while failing",
            "state": "recovered",
            "effect": "clear",
            "evidence": "R4 scenario recovery resets the streak; design D5 later success clears the flag"
          },
          {
            "input": "success while clean",
            "state": "clean",
            "effect": "no-op",
            "evidence": "design D5"
          }
        ],
        "forbidden": [
          "a toast emitted on a non-transition failure (second consecutive failure)",
          "a failure with neither a toast-transition nor a log::warn!/log::error! record (R4 requires logging every failure)",
          "toast text containing a subscription URL or node credential — subscription display name and error string only (R3)",
          "streak state cleared by anything other than a success of the same source"
        ],
        "seeding": [
          "pure: FailureStreak::new() then record_failure/record_success; no GTK objects in tests",
          "geodata path: refresh_outcome_toast(&mut streak, &Ok(())) / &Err(String) values",
          "subscriptions path: the helper over &mut HashMap<uuid::Uuid, FailureStreak> with (id, name, Result) — seeded by insertion only"
        ],
        "budgets": [
          "exactly 1 toast per failure streak (transition count = 1)",
          "scheduling unchanged: geodata loop period interval_secs, subscription tick >= 60 s (subscriptions.rs:1084)",
          "auto_update_streaks memory grows with the subscription count only"
        ]
      },
      "codeTasks": [
        "test geodata_failure_toasts_once_per_streak: drive refresh_outcome_toast — fail -> Some, fail -> None, fail -> None, success -> cleared, fail -> Some again",
        "test subscription_failure_toasts_once_per_streak: same shape through the subscriptions helper; assert the toast text names the subscription",
        "implement crates/ui/src/failure_streak.rs (FailureStreak, record_failure -> bool, record_success) and pub(crate) mod failure_streak in lib.rs",
        "geodata: extend GeodataRefreshService::spawn with the UnboundedSender<String> param, add refresh_outcome_toast, use it in run_loop keeping the per-failure log::warn!",
        "app.rs: create the channel at service spawn (app.rs:869) and add the forwarder task mapping messages to AppMsg::ShowToast",
        "subscriptions: auto_update_streaks field on the model init, transition toasts via SubscriptionsOutput::Notice in AutoUpdateDone (subscriptions.rs:992-1028), record_success on Ok",
        "run make test-ui and cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings"
      ]
    },
    {
      "id": "verify-docs-live",
      "tasks": [
        "5.1",
        "5.2",
        "5.3"
      ],
      "summary": "NO-RED-WAIVER: Rust seam, no test-writer agent; tests are written by rust-coder as its first codeTasks. NO-TESTER-WAIVER: redRun is cargo test, not a go test command; the chunk closes by waiver with tests run in-slice. This seam owns no new code contract — workspace-wide proof, the live SEGV reproduction and documentation. 5.1: full workspace suite. 5.2 guided-live (needs a display and a real backend binary): launch the dev profile, connect a node, kill -SEGV the backend pid read from the dev runtime backend.pid (pid_file_path = runtime_dir/backend.pid, crates/core/src/persistence/mod.rs:224-226), quit the app, relaunch, then assert that the dev backend.log (~/.local/state/v2ray-rs-dev/logs/backend.log) still holds the backend last output lines and an exit requested=false signal=11 record — the exact 2026-09-11 restart-loop forensic that motivated the change (R2 scenario crash reason readable after restart). GUI steps are interactive; the file assertions are scripted greps. 5.3: CHANGELOG.md [Unreleased] Added entries and a docs/ARCHITECTURE.md logging section covering <state_dir>/logs/v2ray-rs.log and <state_dir>/logs/backend.log, 5 MiB rotation with 3 rotated files, 0600/0700 privacy, V2RAY_RS_LOG, session/exit record fields, and the once-per-streak failure toasts.",
      "contract": {
        "states": [
          "undocumented",
          "changelog-entry",
          "docs-section",
          "live-verified"
        ],
        "transitions": [
          {
            "input": "make test TEST_TIMEOUT=10m green",
            "state": "live-verified",
            "effect": "set",
            "evidence": "task 5.1"
          },
          {
            "input": "SEGV reproduction observed in relaunched backend.log",
            "state": "live-verified",
            "effect": "set",
            "evidence": "task 5.2; R2 scenario"
          },
          {
            "input": "CHANGELOG.md [Unreleased] Added entries written",
            "state": "changelog-entry",
            "effect": "set",
            "evidence": "task 5.3"
          },
          {
            "input": "docs/ARCHITECTURE.md logging section written",
            "state": "docs-section",
            "effect": "set",
            "evidence": "task 5.3"
          },
          {
            "input": "any workspace suite red",
            "state": "undocumented",
            "effect": "clear",
            "evidence": "task 5.1 gate"
          }
        ],
        "forbidden": [
          "docs naming paths or env vars with spelling that differs from the implementation (V2RAY_RS_LOG, v2ray-rs.log, backend.log, <state_dir>/logs)",
          "5.2 claimed from a requested stop — only an unrequested signal exit proves the record",
          "5.1 run with fewer than --test-threads=4 or without the 10 m timeout wrapper"
        ],
        "seeding": [
          "live states need the dev profile launched with a display and a real backend installed; scripted parts: kill -SEGV the pid from the dev runtime backend.pid, then grep backend.log for the exit record"
        ],
        "budgets": [
          "make test TEST_TIMEOUT=10m = timeout 10m cargo test --workspace --all-targets -- --test-threads=4",
          "make lint = cargo fmt -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings"
        ]
      },
      "codeTasks": [
        "5.1: run make test TEST_TIMEOUT=10m; fix fallout only in crates touched by this change",
        "5.2 guided-live: cargo run -p v2ray-rs-ui -- --profile development, connect, kill -SEGV the pid from the dev runtime backend.pid, quit, relaunch, verify backend.log tail plus exit requested=false signal=11 (interactive GUI steps, scripted file assertions)",
        "5.3: CHANGELOG.md [Unreleased] Added entries for rotating application/backend logs, session/exit records, V2RAY_RS_LOG, once-per-streak failure toasts; docs/ARCHITECTURE.md section with exact paths and budgets",
        "verify docs: rg -n 'V2RAY_RS_LOG|logs/v2ray-rs.log|logs/backend.log' CHANGELOG.md docs/ARCHITECTURE.md returns hits in both files"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "The system SHALL write every application log record at `info` level and above",
      "tests": [
        "app_logger_writes_warn_and_filters_debug_by_default",
        "apppaths_logs_dir_per_profile"
      ]
    },
    {
      "shall": "The level SHALL be overridable at launch through an environment variable",
      "tests": [
        "log_level_resolves_v2ray_rs_log"
      ]
    },
    {
      "shall": "the record SHALL appear in `<state_dir>/logs/v2ray-rs.log` with a timestamp, level, and source",
      "tests": [
        "app_logger_writes_warn_and_filters_debug_by_default"
      ]
    },
    {
      "shall": "its records SHALL be written under the development profile's state directory",
      "tests": [
        "apppaths_logs_dir_per_profile"
      ]
    },
    {
      "shall": "The system SHALL append every line the backend writes to stdout or stderr",
      "tests": [
        "backend_log_captures_all_lines_under_load"
      ]
    },
    {
      "shall": "Each backend launch SHALL be preceded by a session record",
      "tests": [
        "session_record_precedes_spawn"
      ]
    },
    {
      "shall": "each backend exit SHALL be followed by an exit record",
      "tests": [
        "exit_record_marks_requested_stop",
        "exit_record_marks_unrequested_crash"
      ]
    },
    {
      "shall": "SHALL still contain the backend's last output lines and an exit record marking the exit as unrequested",
      "tests": [
        "exit_record_marks_unrequested_crash"
      ]
    },
    {
      "shall": "every line SHALL still be written to the backend log file",
      "tests": [
        "backend_log_captures_all_lines_under_load"
      ]
    },
    {
      "shall": "The system SHALL rotate each log file when it reaches 5 MiB",
      "tests": [
        "rotating_writer_rotates_at_threshold",
        "rotating_writer_keeps_at_most_three_rotations"
      ]
    },
    {
      "shall": "Log files SHALL be readable and writable only by the owning user",
      "tests": [
        "rotating_writer_creates_private_file_and_dir"
      ]
    },
    {
      "shall": "Application log records SHALL NOT contain subscription URLs or node credentials",
      "tests": [
        "log-site audit (ui-audit codeTask: rg -n audit re-run reviewed against R3)"
      ]
    },
    {
      "shall": "it SHALL be renamed to `.1`, older rotations shifted up",
      "tests": [
        "rotating_writer_rotates_at_threshold"
      ]
    },
    {
      "shall": "it SHALL have mode 0600 and its directory mode 0700",
      "tests": [
        "rotating_writer_creates_private_file_and_dir"
      ]
    },
    {
      "shall": "the record SHALL identify the subscription or node by name and SHALL NOT include its URL, UUID, or password",
      "tests": [
        "log-site audit (ui-audit codeTask: rg -n audit re-run reviewed against R3)"
      ]
    },
    {
      "shall": "The system SHALL notify the user with a toast when a geodata refresh fails or a subscription auto-update fails, once per failure streak",
      "tests": [
        "geodata_failure_toasts_once_per_streak",
        "subscription_failure_toasts_once_per_streak"
      ]
    },
    {
      "shall": "a toast SHALL name the subscription and the failure",
      "tests": [
        "subscription_failure_toasts_once_per_streak"
      ]
    },
    {
      "shall": "no new toast SHALL be shown, and the failure SHALL still be logged",
      "tests": [
        "geodata_failure_toasts_once_per_streak",
        "subscription_failure_toasts_once_per_streak"
      ]
    },
    {
      "shall": "that later failure SHALL toast again",
      "tests": [
        "geodata_failure_toasts_once_per_streak",
        "subscription_failure_toasts_once_per_streak"
      ]
    }
  ],
  "testHarness": [
    "test_paths() — crates/core/src/persistence/mod.rs:462 — creates TempDir + AppPaths::for_profile_in(Test, root); used by all core persistence tests",
    "MockEnv — crates/core/src/persistence/mod.rs:473 — HashMap-based Env impl for testing XDG env resolution",
    "for_profile_in() — crates/core/src/persistence/mod.rs:104 — public test constructor building AppPaths from a root dir",
    "write_script() — crates/process/src/manager.rs:684 — writes a shell script to a TempDir, sets 0755; used by all manager tests",
    "manager_for() — crates/process/src/manager.rs:691 — builds ProcessManager from TempDir + script body; creates config.json + binary + pid path",
    "stub_helper() — crates/process/src/manager.rs:751 — writes a stub netctl helper script, returns (helper_path, calls_log_path)",
    "xray_on_lo() — crates/process/src/manager.rs:765 — builds TunRuntime pointing at loopback with stub helper",
    "drain_states() — crates/process/src/manager.rs:928 — drains broadcast receiver into Vec<ProcessState> for assertion",
    "create_test_subscription() — crates/ui/src/subscriptions.rs:2210 — builds a Subscription with given name/nodes for UI tests",
    "geo_rule() — crates/ui/src/geodata_service.rs:88 — builds RoutingRule with given RuleMatch for geodata tests"
  ],
  "floor": "make lint && make test TEST_TIMEOUT=10m",
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
