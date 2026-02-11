# Tasks: Process Management

## 1. Process State Machine

- [x] 1.1 Define ProcessState enum (Stopped, Starting, Running, Stopping, Error)
- [x] 1.2 Implement state transition validation (only valid transitions allowed)
- [x] 1.3 Define ProcessEvent enum for state change notifications
- [x] 1.4 Implement event channel (tokio broadcast) for state change subscribers

## 2. Process Lifecycle

- [x] 2.1 Implement process start: spawn backend binary with config path argument using tokio::process::Command
- [x] 2.2 Implement stdout/stderr pipe capture with async line-by-line reading
- [x] 2.3 Implement graceful stop: SIGTERM → 5s timeout → SIGKILL using nix crate
- [x] 2.4 Implement restart: stop + start with state transitions
- [x] 2.5 Handle binary-not-found and config-missing error cases
- [x] 2.6 Write PID file on process start, remove on stop

## 3. Log Management

- [x] 3.1 Implement circular log buffer (10,000 lines max)
- [x] 3.2 Implement log line event emission for live UI streaming
- [x] 3.3 Implement log buffer read access for UI (get last N lines, search)

## 4. Crash Recovery

- [x] 4.1 Implement unexpected exit detection (process exits while state is Running)
- [x] 4.2 Implement auto-restart with 2-second delay on crash
- [x] 4.3 Implement crash counter: 3 consecutive crashes within 1 minute → Error state
- [x] 4.4 Implement PID file check on app startup (kill orphaned process if found)

## 5. App Exit Cleanup

- [x] 5.1 Register shutdown handler to send SIGTERM to backend on app exit
- [x] 5.2 Wait for process exit (with timeout) during app shutdown
- [x] 5.3 Write tests for lifecycle scenarios (start, stop, restart, crash recovery)
