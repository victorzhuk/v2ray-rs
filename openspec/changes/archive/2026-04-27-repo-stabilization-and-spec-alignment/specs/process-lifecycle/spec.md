## MODIFIED Requirements

### Requirement: Start backend process
The system SHALL emit an `Error` process state when pre-launch validation fails.

#### Scenario: Binary not found
- **WHEN** the configured binary path does not exist
- **THEN** the process manager SHALL emit `Starting` followed by `Error`, then return the validation error

#### Scenario: Config file missing
- **WHEN** the config file does not exist
- **THEN** the process manager SHALL emit `Starting` followed by `Error`, then return the validation error

