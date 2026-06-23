## MODIFIED Requirements

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

## ADDED Requirements

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
