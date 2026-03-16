# Spec: Process Lifecycle

## ADDED Requirements

### Requirement: Start backend process
The system SHALL launch the selected backend binary using the invocation `binary run -c <config_path>`.

#### Scenario: Successful start
- **WHEN** the user initiates a connection
- **THEN** the system SHALL spawn the backend process, transition state to Running, and begin capturing output

#### Scenario: Binary not found
- **WHEN** the configured binary path does not exist
- **THEN** the system SHALL transition to Error state with a descriptive message

#### Scenario: Config file missing
- **WHEN** the config file does not exist at the expected path
- **THEN** the system SHALL transition to Error state with a `ConfigMissing` error (config generation is the caller's responsibility before invoking `start()`)

### Requirement: Stop backend process
The system SHALL gracefully stop the running backend process using SIGTERM, falling back to SIGKILL after a timeout.

#### Scenario: Graceful stop
- **WHEN** the user disconnects
- **THEN** the system SHALL send SIGTERM, wait up to 5 seconds for exit, then send SIGKILL if still running

#### Scenario: Already stopped
- **WHEN** stop is requested but no process is running
- **THEN** the system SHALL return `Ok(())` silently and remain in Stopped state

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
The system SHALL ensure the backend process is terminated when the application exits.

#### Scenario: Normal app exit
- **WHEN** the user quits the application
- **THEN** the system SHALL send SIGTERM to the backend process and wait for it to exit before completing shutdown

#### Scenario: PID file for crash recovery
- **WHEN** the app starts and finds a PID file from a previous run
- **THEN** the system SHALL check if that process is still running and kill it if so

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
