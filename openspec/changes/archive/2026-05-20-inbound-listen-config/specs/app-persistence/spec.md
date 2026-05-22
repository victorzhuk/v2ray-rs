## MODIFIED Requirements

### Requirement: Settings persistence
The system SHALL serialize application settings to TOML format and store them in the config directory. The system SHALL deserialize settings on startup. When a newly introduced field is absent from a previously written `settings.toml`, the system SHALL fall back to that field's documented default without prompting the user.

#### Scenario: Save and reload settings
- **WHEN** the user changes a setting and the app restarts
- **THEN** the changed setting SHALL be preserved

#### Scenario: Corrupt config handling
- **WHEN** the config file is corrupted or unparseable
- **THEN** the system SHALL fall back to defaults and warn the user

#### Scenario: Missing listen_address in legacy settings
- **WHEN** an existing `settings.toml` does not contain a `listen_address` field
- **THEN** the system SHALL load the settings successfully with `listen_address = "127.0.0.1"` and SHALL NOT log an error

#### Scenario: Round-trip non-loopback listen address
- **WHEN** the user sets `listen_address = "0.0.0.0"` and the settings are saved and reloaded
- **THEN** the reloaded `AppSettings` SHALL contain `listen_address = "0.0.0.0"`
