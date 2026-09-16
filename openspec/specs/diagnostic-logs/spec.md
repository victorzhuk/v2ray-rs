# diagnostic-logs Specification

## Purpose
Keeps a durable, bounded, private record of what the application and its backend did, so failures such as backend crashes can be diagnosed after the fact, and makes background failures visible to the user.

## Requirements

### Requirement: Application log records are persisted
The system SHALL write every application log record at `info` level and above to a log file under the active profile's state directory and to standard error, starting before persistence is initialized. The level SHALL be overridable at launch through an environment variable.

#### Scenario: Warning reaches the file
- **WHEN** any component logs a warning
- **THEN** the record SHALL appear in `<state_dir>/logs/v2ray-rs.log` with a timestamp, level, and source

#### Scenario: Profiles keep separate logs
- **WHEN** the development profile is running
- **THEN** its records SHALL be written under the development profile's state directory, not the production one

### Requirement: Backend output survives the application
The system SHALL append every line the backend writes to stdout or stderr, and every line the route helper writes — including during a TUN route-recovery pass run outside a connection — to a backend log file under the state directory. Each backend launch SHALL be preceded by a session record naming the backend, its version, the node, whether TUN is on, the TUN DNS hijack mode, whether port-53 capture is installed, whether strict routing is on, whether every node hostname was pinned to addresses, and whether the node's routing and DNS came from an imported profile or the app settings. Each backend exit SHALL be followed by an exit record stating the exit code or signal, whether the stop was requested, the reason for the exit (`user-stop`, `node-switch`, `apply-restart`, `app-quit`, `health-failover`, `start-failed`, or `crash`; `health-failover` SHALL mark a stop the application made to reconnect away from an unhealthy node; `start-failed` SHALL cover both a start the application aborted and a backend that exited on its own before it was ready, and in the latter case the record SHALL read `requested=false`), the number of crashes in the current window, the last output line, and the last warning, error, or fatal line among the recent output (or `none`). When the application starts and finds a backend PID file left by a previous run, it SHALL append a record stating that the previous run ended without stopping its backend and whether a still-running backend was killed. A route-recovery pass SHALL additionally record its outcome (success, exit status, or timeout).

#### Scenario: Crash reason is readable after restart
- **WHEN** the backend crashes and the application is later restarted
- **THEN** `<state_dir>/logs/backend.log` SHALL still contain the backend's last output lines and an exit record marking the exit as unrequested with reason `crash`

#### Scenario: Lines are not lost under load
- **WHEN** the backend writes lines faster than the user interface consumes them
- **THEN** every line SHALL still be written to the backend log file

#### Scenario: Recovery output is kept
- **WHEN** a TUN route-recovery pass runs at startup or after a session ended without a clean stop
- **THEN** every line the route helper printed and the pass's outcome SHALL appear in `<state_dir>/logs/backend.log`, and none SHALL be written only to the application's standard error

#### Scenario: Stop reasons are distinguished
- **WHEN** the user disconnects, then later applies pending settings with restart, then switches to another node, then quits the application
- **THEN** the four exit records SHALL carry reasons `user-stop`, `apply-restart`, `node-switch`, and `app-quit` respectively

#### Scenario: Health failover is not a user stop
- **WHEN** health failover is on and the application stops an unhealthy session to reconnect without that node
- **THEN** the exit record SHALL carry `requested=true` and reason `health-failover`

#### Scenario: Failed start is not a user stop
- **WHEN** the backend is stopped because its TUN device did not appear or the route helper failed during launch
- **THEN** the exit record SHALL carry reason `start-failed`

#### Scenario: Backend that exits before ready is a failed start
- **WHEN** the backend exits on its own during the initial start, before it has been reported ready
- **THEN** the exit record SHALL carry `requested=false`, reason `start-failed`, and `crashes_in_window=0`

#### Scenario: Last error survives access noise
- **WHEN** the backend's last output lines are access-log lines preceded by a `[Warning]` line
- **THEN** the exit record SHALL carry that warning line as `last_error` and the final access line as `last_output`

#### Scenario: Session record carries TUN DNS decisions
- **WHEN** an xray TUN session starts with DNS hijack mode `hijack`, every node hostname pinned, and the app's own settings
- **THEN** the session record SHALL state `hijack=hijack`, `capture_dns=true`, `nodes_pinned=true`, and `profile=app`

#### Scenario: Unclean previous run is recorded
- **WHEN** the application starts and a backend PID file from the previous run exists
- **THEN** `backend.log` SHALL contain a record stating the previous run ended without stopping its backend, before any new session record

### Requirement: Log files are bounded and private
The system SHALL rotate each log file when it reaches 5 MiB, keeping at most three rotated files per log. Log files SHALL be readable and writable only by the owning user, in a directory accessible only by that user. Application log records SHALL NOT contain subscription URLs or node credentials.

#### Scenario: Rotation
- **WHEN** a log file reaches 5 MiB
- **THEN** it SHALL be renamed to `.1`, older rotations shifted up, the fourth-oldest deleted, and writing continued in a new file

#### Scenario: Permissions
- **WHEN** a log file is created
- **THEN** it SHALL have mode 0600 and its directory mode 0700

#### Scenario: No credentials in application records
- **WHEN** a subscription update or connection attempt is logged
- **THEN** the record SHALL identify the subscription or node by name and SHALL NOT include its URL, UUID, or password

### Requirement: Background failures are visible
The system SHALL notify the user with a toast when a geodata refresh fails or a subscription auto-update fails, once per failure streak, and SHALL log every such failure.

#### Scenario: First failure toasts
- **WHEN** a scheduled subscription auto-update fails after the previous attempt succeeded
- **THEN** a toast SHALL name the subscription and the failure

#### Scenario: Repeated failure stays quiet
- **WHEN** the next scheduled attempt fails again
- **THEN** no new toast SHALL be shown, and the failure SHALL still be logged

#### Scenario: Recovery resets the streak
- **WHEN** an attempt succeeds after failures and a later attempt fails
- **THEN** that later failure SHALL toast again

### Requirement: Connection decisions are logged
The application log SHALL record, at `info` level or above: each connection attempt with its origin (`user`, `node`, `auto-reconnect`, or `restart`), the resolve strategy, the number of candidates, and whether any candidate uses an imported profile; each candidate start with its position and node name; each candidate failure with its position, node name, and failure reason before the next candidate starts; each auto-reconnect scheduled with its attempt number and limit, and when the limit is exhausted; each backend state transition; each crash respawn with its crash count; and each quit request. Records SHALL identify nodes by name and SHALL NOT include credentials.

#### Scenario: Failover is traceable
- **WHEN** a connection with three candidates fails on the first and connects on the second
- **THEN** the app log SHALL contain the connect record with `candidates=3`, a failure record for candidate 1 with its reason, and a start record for candidate 2, in that order

#### Scenario: Auto-reconnect is traceable
- **WHEN** a connection ends in `Error` and an auto-reconnect is scheduled
- **THEN** the app log SHALL contain a record with the attempt number and the limit of 3, followed by a connect record with origin `auto-reconnect` when it fires

#### Scenario: Crash respawn is traceable
- **WHEN** the backend exits unexpectedly and is respawned in place
- **THEN** the app log SHALL contain a crash-respawn record with the exit code or signal and the crash count, and state-transition records for `Running` → `Starting` → `Running`
