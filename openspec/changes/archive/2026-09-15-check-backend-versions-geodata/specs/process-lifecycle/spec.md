## ADDED Requirements

### Requirement: sing-box minimum version is checked before spawn
The system SHALL require sing-box 1.13.0 or newer, the oldest version the generated config targets. Starting a sing-box connection with an older installed version SHALL fail before the backend is spawned, with an error naming the installed and required versions, and SHALL end the connection attempt without trying further candidates. When the installed version cannot be read, the start SHALL proceed and a warning SHALL be written to the process log stream stating that the version could not be read and the minimum-version check was skipped.

#### Scenario: Old sing-box blocked
- **WHEN** the selected backend is sing-box and its reported version is 1.12.4
- **THEN** Connect SHALL fail without spawning a backend, and the error SHALL name 1.12.4 and 1.13.0

#### Scenario: Supported sing-box starts
- **WHEN** the selected backend is sing-box and its reported version is 1.13.0 or newer
- **THEN** the start SHALL proceed to the config check

#### Scenario: Unreadable sing-box version
- **WHEN** the sing-box `version` output cannot be read or parsed
- **THEN** the start SHALL proceed and the process log SHALL contain a warning that the minimum-version check was skipped
