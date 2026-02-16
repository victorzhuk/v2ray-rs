## MODIFIED Requirements

### Requirement: Start backend process
The system SHALL launch the selected backend binary with the generated config file path as a command-line argument.

#### Scenario: Successful start
- **WHEN** the user initiates a connection with auto-resolve enabled
- **THEN** the system SHALL spawn the backend process using the active candidate config, transition state to Running, and begin capturing output

#### Scenario: Binary not found
- **WHEN** the configured binary path does not exist
- **THEN** the system SHALL transition to Error state with a descriptive message

#### Scenario: Config file missing
- **WHEN** the config file does not exist at the expected path
- **THEN** the system SHALL generate it first, then start the process

### Requirement: Process state reporting
The system SHALL expose current process state and active connection metadata to other components via events.

#### Scenario: State change notification
- **WHEN** the process state changes
- **THEN** the system SHALL emit an event that includes connection metadata for the UI and tray to update their display
