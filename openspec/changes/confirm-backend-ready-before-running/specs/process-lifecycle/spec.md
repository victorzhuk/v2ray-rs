## ADDED Requirements

### Requirement: Running is reported only once the backend is ready
The system SHALL report a backend start, and an in-place respawn, as `Running` only after the backend is ready: its local SOCKS (or mixed) inbound accepts a TCP connection and the process is still running at least one second after it was spawned. An unspecified listen address SHALL be probed on the loopback address of the same family. For xray in TUN mode, readiness SHALL be checked after the TUN device has appeared and the route helper has programmed the routes. When the backend does not become ready within 15 seconds, the system SHALL stop it and fail the start with an error naming the probed address and the timeout. When the backend exits before it is ready during the initial start of a candidate, the start SHALL fail with an error stating the exit code or signal and the backend's last output line; the system SHALL NOT respawn it and SHALL NOT record a crash.

#### Scenario: Backend that FATALs after binding its inbound
- **WHEN** a backend binds its inbound and exits with code 1 about 100 ms after spawn
- **THEN** the connection SHALL NOT be reported `Running`, no respawn SHALL be attempted for that candidate, and the start SHALL fail with a reason containing the exit code and the last output line

#### Scenario: Healthy start
- **WHEN** the backend's inbound accepts connections and the process is still running one second after spawn
- **THEN** the system SHALL report `Running` with the connection metadata

#### Scenario: Backend never listens
- **WHEN** the backend stays alive but its inbound does not accept connections within 15 seconds
- **THEN** the system SHALL stop the backend and fail the start with an error naming the address and the timeout

#### Scenario: Startup failure fails over
- **WHEN** a candidate's backend exits before it is ready and another candidate remains
- **THEN** the connection SHALL try the next candidate without waiting for a crash-restart delay

#### Scenario: xray TUN readiness follows route setup
- **WHEN** xray starts in TUN mode
- **THEN** the system SHALL wait for the TUN device and run the route helper before checking readiness, and SHALL report `Running` only after all three succeed

#### Scenario: Disconnect while waiting for readiness
- **WHEN** the user invokes Disconnect while the backend is spawned but not yet ready
- **THEN** the system SHALL stop the backend, release TUN routing state, and report `Stopped`

## MODIFIED Requirements

### Requirement: Crash detection and recovery
The system SHALL detect unexpected exits of a backend that has been reported `Running` and restart it automatically within a bounded budget. An exit before the backend was first ready is a startup failure, not a crash, and SHALL NOT be restarted in place. A respawn that fails to start, including one that exits or times out before it is ready, SHALL count as a crash and SHALL be retried while the budget allows. An in-place respawn SHALL reuse the pre-launch validation of the start it replaces, relaunching the same binary with the same config without re-probing the version, the capabilities, or the config. For xray in TUN mode the respawn SHALL NOT remove the session's routing state.

#### Scenario: Single crash
- **WHEN** the backend process exits unexpectedly after it was reported `Running`
- **THEN** the system SHALL wait 2 seconds and attempt to restart automatically

#### Scenario: Repeated crashes
- **WHEN** the backend process exits unexpectedly 3 or more times within a 1-minute sliding window
- **THEN** the system SHALL transition to Error state instead of restarting. Any exit while the backend is expected to be running counts as a crash, including a signal death (OOM, segfault, external kill); a requested stop moves the state to `Stopping` first and never reaches crash handling.

#### Scenario: Exit during initial start is not a crash
- **WHEN** the backend exits before it was ever ready during a candidate's initial start
- **THEN** the system SHALL NOT wait, SHALL NOT respawn, SHALL NOT count a crash, and SHALL fail the start with the exit reason

#### Scenario: Failed respawn is retried
- **WHEN** a respawn attempt fails to start (for example the TUN device does not appear, the route helper fails, or the respawned backend exits before it is ready) and fewer than 3 crashes are recorded in the window
- **THEN** the failure SHALL be recorded as a crash and the system SHALL wait and respawn again instead of transitioning to Error

#### Scenario: Respawn skips redundant preflight
- **WHEN** the system respawns the backend after an unexpected exit
- **THEN** it SHALL relaunch without re-running the backend version probe, the capability probe, or the backend's config check

#### Scenario: xray TUN routing state survives a respawn
- **WHEN** an xray TUN backend exits unexpectedly and is respawned
- **THEN** the policy rules installed for the session SHALL remain in place from the exit until the respawned backend's routes are programmed
