## Why

The logs cannot answer "why did it reconnect" or "why did it stop". In `backend.log` from 2026-09-13 to 2026-09-15, all 59 xray exit records say `requested=true` whether the user disconnected, a settings apply restarted the session, a node switch replaced it, or the app quit. The app log never records how a connection was started (origin, strategy, candidate count, imported-profile override), which candidate failed and why before failover, when an auto-reconnect was scheduled, or backend state changes. A crash respawn shows in the status bar as an ordinary "Connecting…". Twice on 2026-09-14 (app starts 07:11:44 and 07:16:45 UTC) a session has no exit record and the app log has no quit or panic line: the app ended without stopping the backend, and nothing says so. And the exit record's `last_output` was a routine access line (`from tcp:… accepted …`) in 44 of 59 requested exits, hiding the last warning or error.

## What Changes

- Exit records gain `reason=` (`user-stop`, `node-switch`, `apply-restart`, `app-quit`, `start-failed`, `crash`) and `last_error=` (the last warning, error, or fatal line, or `none`). `requested=` and `last_output=` stay.
- Session records gain the TUN DNS decisions: `hijack=`, `capture_dns=`, `strict=`, `nodes_pinned=`, and `profile=imported|app`.
- The app log records each connect (`origin`, `strategy`, `candidates`, `profile_override`), each candidate start and failure with its reason, each auto-reconnect scheduled or exhausted, each backend state transition, each crash respawn, quit requests, and termination signals.
- SIGTERM, SIGINT, and SIGHUP run the normal quit path, so the backend is stopped and its exit record written.
- A panic is written to the app log before the process aborts.
- At startup, a PID file left by a previous run is recorded in `backend.log` as an unclean exit of that run.
- The status bar shows "Restarting after crash" during an in-place respawn and "Reconnecting (n/3)" during an auto-reconnect.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `diagnostic-logs`: "Backend output survives the application" gains exit reason, last error, and session DNS fields; new requirement for connection decisions in the app log.
- `process-lifecycle`: new requirement for termination signals and panics.
- `main-window`: new requirement for respawn and reconnect status text.

## Impact

- `crates/process/src/manager.rs` — stop reason, `last_error`, session fields, crash-respawn log line.
- `crates/process/src/state.rs` — transition log line.
- `crates/ui/src/connection.rs` — `ConnectionCmd::Stop` carries a reason; candidate start/failure lines; session fields.
- `crates/ui/src/app.rs` — connect/auto-reconnect/quit lines, stop reasons, origin tracking for the status bar, signal handlers, panic hook, unclean-exit record.
- Relies on the existing `log` records being persisted by `crates/ui/src/logging.rs`.
