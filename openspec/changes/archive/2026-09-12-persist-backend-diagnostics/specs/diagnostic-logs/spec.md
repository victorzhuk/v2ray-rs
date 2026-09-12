## Purpose

Keeps a durable, bounded, private record of what the application and its backend did, so failures such as backend crashes can be diagnosed after the fact, and makes background failures visible to the user.

## ADDED Requirements

### Requirement: Application log records are persisted
The system SHALL write every application log record at `info` level and above to a log file under the active profile's state directory and to standard error, starting before persistence is initialized. The level SHALL be overridable at launch through an environment variable.

#### Scenario: Warning reaches the file
- **WHEN** any component logs a warning
- **THEN** the record SHALL appear in `<state_dir>/logs/v2ray-rs.log` with a timestamp, level, and source

#### Scenario: Profiles keep separate logs
- **WHEN** the development profile is running
- **THEN** its records SHALL be written under the development profile's state directory, not the production one

### Requirement: Backend output survives the application
The system SHALL append every line the backend writes to stdout or stderr, and every line the route helper writes, to a backend log file under the state directory. Each backend launch SHALL be preceded by a session record naming the backend, its version, the node, and whether TUN is on, and each backend exit SHALL be followed by an exit record stating the exit code or signal, whether the stop was requested, the number of crashes in the current window, and the last output line.

#### Scenario: Crash reason is readable after restart
- **WHEN** the backend crashes and the application is later restarted
- **THEN** `<state_dir>/logs/backend.log` SHALL still contain the backend's last output lines and an exit record marking the exit as unrequested

#### Scenario: Lines are not lost under load
- **WHEN** the backend writes lines faster than the user interface consumes them
- **THEN** every line SHALL still be written to the backend log file

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
