# Spec: Backend Detection

## ADDED Requirements

### Requirement: Auto-detect installed backends
The system SHALL keep detected backend binaries visible even when version probing fails, marking them unavailable instead of silently omitting the failure.

#### Scenario: Version probe failure remains visible
- **WHEN** a backend binary exists but `version` probing fails
- **THEN** the backend remains listed in onboarding/preferences, is disabled for selection, and displays the probe error

#### Scenario: Single usable backend installed
- **WHEN** exactly one detected backend is available for use
- **THEN** onboarding SHALL auto-select that backend

### Requirement: Backend version detection
The system SHALL query each detected backend's version by executing the binary with appropriate arguments and parsing stdout.

#### Scenario: Successful version query
- **WHEN** `/usr/bin/v2ray` is detected
- **THEN** the system SHALL run `v2ray version` and store the version string

#### Scenario: Binary exists but fails to run
- **WHEN** a binary exists but returns an error on version query
- **THEN** the system SHALL mark that backend as unavailable with the error message

### Requirement: Custom backend path
The system SHALL validate custom backend paths strictly before accepting them.

#### Scenario: Version probe fails for custom path
- **WHEN** the user enters an executable custom path whose `version` command fails
- **THEN** the system SHALL reject the path and show the validation error instead of saving it

### Requirement: Backend selection persistence
The system SHALL persist the user's backend selection so it survives app restarts.

#### Scenario: Restart preserves selection
- **WHEN** the user selects sing-box and restarts the app
- **THEN** sing-box SHALL remain the selected backend
