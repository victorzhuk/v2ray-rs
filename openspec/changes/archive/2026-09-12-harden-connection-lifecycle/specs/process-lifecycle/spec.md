## MODIFIED Requirements

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

## ADDED Requirements

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
