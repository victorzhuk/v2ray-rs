# Spec: Process Lifecycle

## Purpose

Defines how the application manages the lifecycle of the backend proxy process: starting and stopping it with graceful signal handling, supervising it for crashes with bounded automatic restart, capturing its logs, reporting state changes, and cleaning up on exit including TUN route recovery.

## Requirements

### Requirement: Start backend process
The system SHALL emit an `Error` process state when pre-launch validation fails.

#### Scenario: Binary not found
- **WHEN** the configured binary path does not exist
- **THEN** the process manager SHALL emit `Starting` followed by `Error`, then return the validation error

#### Scenario: Config file missing
- **WHEN** the config file does not exist
- **THEN** the process manager SHALL emit `Starting` followed by `Error`, then return the validation error

### Requirement: Stop backend process
The system SHALL gracefully stop the running backend process using SIGTERM, falling back to SIGKILL after a timeout.

#### Scenario: Graceful stop
- **WHEN** the user disconnects
- **THEN** the system SHALL send SIGTERM, wait up to 5 seconds for exit, then send SIGKILL if still running

#### Scenario: Already stopped
- **WHEN** stop is requested but no process is running
- **THEN** the system SHALL return `Ok(())` silently and remain in Stopped state

#### Scenario: Stop while in Error with no child
- **WHEN** stop or shutdown is requested while the manager is in the `Error` state with no running child
- **THEN** the system SHALL transition to `Stopped` and return `Ok(())` instead of leaving the manager parked in `Error`

### Requirement: Restart backend process
The system SHALL support restarting the backend process (stop then start) when config is regenerated or the user requests it.

#### Scenario: Config-triggered restart
- **WHEN** the config file is regenerated
- **THEN** the system SHALL stop the current process and start a new one with the updated config

#### Scenario: Manual restart
- **WHEN** the user requests a restart
- **THEN** the system SHALL perform a graceful stop followed by a start

### Requirement: Log capture
The system SHALL capture stdout and stderr from the backend process and make log lines available to the UI in real-time.

#### Scenario: Live log streaming
- **WHEN** the backend process writes to stdout or stderr
- **THEN** the system SHALL capture each line and make it available to the UI within 100ms

#### Scenario: Log buffer limit
- **WHEN** the log buffer exceeds 10,000 lines
- **THEN** the oldest lines SHALL be discarded to maintain the buffer size

### Requirement: Crash detection and recovery
The system SHALL detect unexpected process exits and optionally attempt automatic restart.

#### Scenario: Single crash
- **WHEN** the backend process exits unexpectedly
- **THEN** the system SHALL wait 2 seconds and attempt to restart automatically

#### Scenario: Repeated crashes
- **WHEN** the backend process exits unexpectedly 3 or more times within a 1-minute sliding window
- **THEN** the system SHALL transition to Error state instead of restarting. Signal-terminated exits (SIGINT/SIGTERM/SIGKILL, codes 130/137/143) are not counted as crashes and go directly to Stopped state.

### Requirement: Process state reporting
The system SHALL expose current process state and active connection metadata to other components via events.

#### Scenario: State change notification
- **WHEN** the process state changes
- **THEN** the system SHALL emit an event that includes connection metadata for the UI and tray to update their display

### Requirement: Cleanup on app exit
The system SHALL ensure the backend process is terminated when the application exits, using a profile-scoped PID file. When an orphaned process does not exit after SIGTERM, the system SHALL escalate to SIGKILL. When the prior run used TUN mode, the system SHALL additionally run a route-recovery pass so that no TUN device or stale routes remain.

#### Scenario: Normal app exit
- **WHEN** the user quits the application
- **THEN** the system SHALL send SIGTERM to the backend process and wait for it to exit before completing shutdown

#### Scenario: PID file for crash recovery
- **WHEN** the app starts and finds a PID file from a previous run at `runtime_dir/backend.pid`
- **THEN** the system SHALL check if that process is still running and kill it if so, escalating from SIGTERM to SIGKILL if it does not exit within the timeout

#### Scenario: PID file does not leak across profiles
- **WHEN** the app launches with one profile while a backend from a different profile is running
- **THEN** the system SHALL only inspect the PID file under the active profile's `runtime_dir` and SHALL NOT touch other profiles' processes

#### Scenario: TUN route recovery after unclean shutdown
- **WHEN** the app starts and the persisted connection state shows the previous run was connected in TUN mode but exited uncleanly
- **THEN** the system SHALL run a route-recovery pass for the relevant backend that removes any leftover TUN device and flushes stale routing rules and tables

### Requirement: Capture launched runtime snapshot
The system SHALL capture an immutable snapshot of the restart-relevant settings and routing rules that were actually used for the current connection attempt.

#### Scenario: Snapshot captured before launch
- **WHEN** the app prepares the config inputs for `Connect`
- **THEN** it stores the exact settings and routing rules passed to config generation before backend start begins

### Requirement: Apply pending runtime changes by restart
The system SHALL apply pending runtime configuration changes by reusing the normal disconnect/reconnect flow.

#### Scenario: Apply and restart while connected
- **WHEN** the user chooses "Apply & Restart" from the restart-required banner
- **THEN** the system disconnects, reconnects with the already-persisted runtime config, and replaces the active runtime snapshot with the new launched snapshot

### Requirement: Single-instance lock per profile
The system SHALL acquire an exclusive advisory lock on `runtime_dir/v2ray-rs.lock` at startup, before initializing persistence or spawning the backend, and SHALL hold it for the lifetime of the process. Two instances of the same profile SHALL NOT run concurrently. Two instances of different profiles SHALL be able to run concurrently because their `runtime_dir`s differ.

#### Scenario: Second instance of the same profile is refused
- **WHEN** an instance is already running for a given profile and a second invocation targets the same profile
- **THEN** the second invocation SHALL fail to acquire the lock, SHALL print the holder PID recorded in `instance.json`, and SHALL exit with code 75

#### Scenario: Different profiles run side-by-side
- **WHEN** an instance is running with `--profile production` and another is started with `--profile development`
- **THEN** both instances SHALL run concurrently without contending for the same lock file

#### Scenario: Lock is released on shutdown
- **WHEN** an instance exits cleanly or is killed
- **THEN** the kernel SHALL release the advisory lock and a subsequent invocation of the same profile SHALL be able to acquire it

### Requirement: TUN-aware connection start and stop
The system SHALL make connection start and stop TUN-aware. For xray with TUN enabled, after spawning the backend the system SHALL wait for the TUN device to appear, bounded by a timeout, then invoke the route helper to program the address and routes before reporting `Running`; if the device does not appear or the helper fails, the system SHALL stop the backend and transition to `Error`. For sing-box with TUN enabled, the backend programs its own routes via `auto_route` and the system SHALL NOT run the route helper. Stop SHALL remain SIGTERM-first so the backend can tear down its own routes before any SIGKILL.

#### Scenario: xray TUN start programs routes
- **WHEN** the user connects with xray and TUN enabled
- **THEN** the system SHALL spawn xray, wait for the TUN device, invoke the route helper to add the split routes, and only then report `Running`

#### Scenario: xray TUN device never appears
- **WHEN** xray is spawned in TUN mode but the device does not appear within the timeout
- **THEN** the system SHALL stop the backend and transition to `Error`

#### Scenario: sing-box TUN start needs no helper
- **WHEN** the user connects with sing-box and TUN enabled
- **THEN** the system SHALL spawn sing-box and rely on its `auto_route` to program routes, without invoking the route helper

#### Scenario: Graceful stop preserves teardown
- **WHEN** the user disconnects from a TUN session
- **THEN** the system SHALL send SIGTERM first so the backend can remove its own routes (sing-box) or close its TUN fd so the kernel drops the routes (xray), escalating to SIGKILL only after the timeout, and for xray SHALL invoke the route-helper teardown as a safeguard
