## ADDED Requirements

### Requirement: Logging settings persistence
The system SHALL persist backend logging preferences in `settings.toml` as a `[logging]` section with `backend_level` (one of `error`, `warning`, `info`, `debug`) and `connection_log` (bool). When the section or a key is absent, the system SHALL load `backend_level = "warning"` and `connection_log = false` without prompting or erroring. Changing either value while connected SHALL count as a restart-relevant change.

#### Scenario: Legacy settings without the section
- **WHEN** an existing `settings.toml` has no `[logging]` section
- **THEN** settings SHALL load with `backend_level = "warning"` and `connection_log = false` and SHALL NOT log an error

#### Scenario: Round-trip
- **WHEN** the user sets the level to `debug` and turns the connection log on, and the app restarts
- **THEN** the reloaded settings SHALL contain `backend_level = "debug"` and `connection_log = true`

#### Scenario: Change while connected
- **WHEN** the user changes the backend log level while connected
- **THEN** the pending-restart indication SHALL appear as for other config-changing settings
