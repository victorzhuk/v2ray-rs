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
The system SHALL gracefully stop the running backend process using SIGTERM, falling back to SIGKILL after a timeout. A stop request SHALL be honored from every process state — including while a crash respawn is waiting or starting, while a connection is starting, and while a route-helper call is in flight — and SHALL always end in a reported `Stopped` state with any TUN routing state released.

#### Scenario: Graceful stop
- **WHEN** the user disconnects
- **THEN** the system SHALL send SIGTERM, wait up to 5 seconds for exit, then send SIGKILL if still running

#### Scenario: Already stopped
- **WHEN** stop is requested but no process is running
- **THEN** the system SHALL return `Ok(())` silently and remain in Stopped state

#### Scenario: Stop while in Error with no child
- **WHEN** stop or shutdown is requested while the manager is in the `Error` state with no running child
- **THEN** the system SHALL transition to `Stopped` and return `Ok(())` instead of leaving the manager parked in `Error`

#### Scenario: Stop during a crash respawn
- **WHEN** the user disconnects while the backend has crashed and the system is waiting to respawn it or is respawning it
- **THEN** the respawn SHALL be abandoned, any backend it spawned SHALL be stopped, TUN routing state SHALL be released, and the system SHALL report `Stopped`

#### Scenario: Stop during start
- **WHEN** the user disconnects while the connection is in the `Starting` state
- **THEN** the system SHALL stop any spawned backend, cancel any in-flight route-helper call, release TUN routing state, and report `Stopped`

#### Scenario: Quit during a stop
- **WHEN** the user quits while a stop is still in progress
- **THEN** the application SHALL wait for the stop to report `Stopped` before exiting, so TUN teardown is not cut short

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
The system SHALL detect unexpected process exits and restart automatically within a bounded budget. A respawn that fails to start SHALL count as a crash and SHALL be retried while the budget allows. An in-place respawn SHALL reuse the pre-launch validation of the start it replaces, relaunching the same binary with the same config without re-probing the version, the capabilities, or the config. For xray in TUN mode the respawn SHALL NOT remove the session's routing state.

#### Scenario: Single crash
- **WHEN** the backend process exits unexpectedly
- **THEN** the system SHALL wait 2 seconds and attempt to restart automatically

#### Scenario: Repeated crashes
- **WHEN** the backend process exits unexpectedly 3 or more times within a 1-minute sliding window
- **THEN** the system SHALL transition to Error state instead of restarting. Any exit while the backend is expected to be running counts as a crash, including a signal death (OOM, segfault, external kill); a requested stop moves the state to `Stopping` first and never reaches crash handling.

#### Scenario: Failed respawn is retried
- **WHEN** a respawn attempt fails to start (for example the TUN device does not appear or the route helper fails) and fewer than 3 crashes are recorded in the window
- **THEN** the failure SHALL be recorded as a crash and the system SHALL wait and respawn again instead of transitioning to Error

#### Scenario: Respawn skips redundant preflight
- **WHEN** the system respawns the backend after an unexpected exit
- **THEN** it SHALL relaunch without re-running the backend version probe, the capability probe, or the backend's config check

#### Scenario: xray TUN routing state survives a respawn
- **WHEN** an xray TUN backend exits unexpectedly and is respawned
- **THEN** the policy rules installed for the session SHALL remain in place from the exit until the respawned backend's routes are programmed

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
The system SHALL capture an immutable snapshot of the restart-relevant settings and routing rules that were actually used for the current connection attempt. Restart-relevant settings SHALL include every setting that changes the generated backend config, including the TUN settings and the connection idle-timeout and WebSocket-heartbeat settings. While connected, UI state derived from those settings SHALL follow the snapshot rather than the edited settings.

#### Scenario: Snapshot captured before launch
- **WHEN** the app prepares the config inputs for `Connect`
- **THEN** it stores the exact settings and routing rules passed to config generation before backend start begins

#### Scenario: TUN indicators follow the running session
- **WHEN** TUN is enabled in settings while a session launched without TUN is running
- **THEN** the app SHALL keep treating the session as non-TUN — background latency probes continue and no TUN recovery marker is written — until the session is restarted with the new settings

### Requirement: Apply pending runtime changes by restart
The system SHALL apply pending runtime configuration changes by reusing the normal disconnect/reconnect flow. When the running session was started by a direct connection to a chosen node, the reconnect SHALL target that node as the sole candidate.

#### Scenario: Apply and restart while connected
- **WHEN** the user chooses "Apply & Restart" from the restart-required banner
- **THEN** the system disconnects, reconnects with the already-persisted runtime config, and replaces the active runtime snapshot with the new launched snapshot

#### Scenario: Apply and restart keeps a directly chosen node
- **WHEN** the running session was started by a direct connection to a node and the user chooses "Apply & Restart"
- **THEN** the system SHALL reconnect to that same node as the sole candidate

#### Scenario: Directly chosen node no longer available
- **WHEN** the user chooses "Apply & Restart" and the directly chosen node has been removed or disabled
- **THEN** the system SHALL notify the user and reconnect using the configured strategy

#### Scenario: TUN edit raises the restart banner
- **WHEN** the user changes any TUN setting, the idle timeout, or the WebSocket heartbeat while connected
- **THEN** the restart-required banner SHALL appear

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
The system SHALL make connection start and stop TUN-aware. For xray with TUN enabled, after spawning the backend the system SHALL wait for the TUN device to appear, bounded by a timeout, then invoke the route helper to program the address and routes before reporting `Running`; if the device does not appear or the helper fails, the system SHALL stop the backend and transition to `Error`. For sing-box with TUN enabled, the backend programs its own routes via `auto_route` and the system SHALL NOT run the route helper. Stop SHALL remain SIGTERM-first so the backend can tear down its own routes before any SIGKILL. Every route-helper invocation SHALL be bounded by a timeout and terminated if its caller is cancelled, and its output SHALL reach the process log stream. The TUN recovery marker SHALL be persisted before the route helper first changes routes, naming the backend and the interface the session actually uses.

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

#### Scenario: Route helper hangs
- **WHEN** a route-helper invocation does not finish within its timeout
- **THEN** the helper process SHALL be killed, the invocation SHALL be treated as failed, and the failure SHALL appear in the process log stream

#### Scenario: Route helper output is visible
- **WHEN** the route helper writes to stdout or stderr, or a teardown invocation fails
- **THEN** that output or failure SHALL appear in the process log stream

#### Scenario: Marker precedes route changes
- **WHEN** an xray TUN connection starts
- **THEN** the TUN recovery marker SHALL be persisted, with the runtime's interface name, before the route helper is first invoked

### Requirement: Connection terminal state has a single source
The system SHALL report a connection's terminal state (`Stopped` or `Error`) only from the component supervising the whole connection attempt, never from an individual candidate's backend. A candidate given up during failover SHALL NOT be reported as the connection stopping.

#### Scenario: Failover is not reported as a stop
- **WHEN** a candidate exhausts its crash budget and the connection fails over to the next candidate
- **THEN** the app SHALL NOT receive `Stopped` for the connection, SHALL keep its connection handle, and SHALL keep the TUN recovery marker

#### Scenario: Disconnect works after failover
- **WHEN** the connection has failed over to another candidate and reached `Running`
- **THEN** Disconnect SHALL stop that backend and report `Stopped`

#### Scenario: Last candidate fails
- **WHEN** every candidate of a connection attempt has failed
- **THEN** the app SHALL receive exactly one terminal `Error` summarizing the failures

### Requirement: In-flight connection can be cancelled
The system SHALL let the user cancel a connection that is starting, including an in-place crash respawn, from the main window and from the tray.

#### Scenario: Cancel a slow connect
- **WHEN** the connection is in the `Starting` state and the user invokes Disconnect
- **THEN** the attempt SHALL be abandoned and the connection SHALL end in `Stopped`

### Requirement: sing-box minimum version is checked before spawn
The system SHALL require sing-box 1.13.0 or newer, the oldest version the generated config targets. Starting a sing-box connection with an older installed version SHALL fail before the backend is spawned, with an error naming the installed and required versions, and SHALL end the connection attempt without trying further candidates. When the installed version cannot be read, the start SHALL proceed and a warning SHALL be written to the process log stream stating that the version could not be read and the minimum-version check was skipped.

#### Scenario: Old sing-box blocked
- **WHEN** the selected backend is sing-box and its reported version is 1.12.4
- **THEN** Connect SHALL fail without spawning a backend, and the error SHALL name 1.12.4 and 1.13.0

#### Scenario: Supported sing-box starts
- **WHEN** the selected backend is sing-box and its reported version is 1.13.0 or newer
- **THEN** the start SHALL proceed to the config check

#### Scenario: Unreadable sing-box version
- **WHEN** the sing-box `version` output cannot be read or parsed
- **THEN** the start SHALL proceed and the process log SHALL contain a warning that the minimum-version check was skipped
