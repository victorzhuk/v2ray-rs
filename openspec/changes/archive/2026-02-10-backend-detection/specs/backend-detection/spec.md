# Spec: Backend Detection

## ADDED Requirements

### Requirement: Auto-detect installed backends
The system SHALL scan for v2ray, xray, and sing-box binaries at well-known paths (`/usr/bin/`, `/usr/local/bin/`) and via the system PATH on startup.

#### Scenario: Single backend installed
- **WHEN** only xray is found at `/usr/bin/xray`
- **THEN** the system SHALL auto-select xray as the active backend

#### Scenario: Multiple backends installed
- **WHEN** both v2ray and sing-box are found
- **THEN** the system SHALL present both options and let the user choose

#### Scenario: No backend installed
- **WHEN** no supported backend binary is found
- **THEN** the system SHALL display an error with installation guidance for each backend

### Requirement: Backend version detection
The system SHALL query each detected backend's version by executing the binary with appropriate arguments and parsing stdout.

#### Scenario: Successful version query
- **WHEN** `/usr/bin/v2ray` is detected
- **THEN** the system SHALL run `v2ray version` and store the version string

#### Scenario: Binary exists but fails to run
- **WHEN** a binary exists but returns an error on version query
- **THEN** the system SHALL mark that backend as unavailable with the error message

### Requirement: Custom backend path
The system SHALL allow the user to specify a custom path to a backend binary, overriding auto-detection.

#### Scenario: User specifies custom path
- **WHEN** the user sets a custom binary path `/opt/xray/xray`
- **THEN** the system SHALL validate that path and use it as the active backend

#### Scenario: Invalid custom path
- **WHEN** the user specifies a path that does not exist or is not executable
- **THEN** the system SHALL reject it with a clear error message

### Requirement: Backend selection persistence
The system SHALL persist the user's backend selection so it survives app restarts.

#### Scenario: Restart preserves selection
- **WHEN** the user selects sing-box and restarts the app
- **THEN** sing-box SHALL remain the selected backend
