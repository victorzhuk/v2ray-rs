# Spec: app-persistence

## ADDED Requirements

### Requirement: XDG-compliant storage paths
The system SHALL store configuration files in `$XDG_CONFIG_HOME/v2ray-rs/` (defaulting to `~/.config/v2ray-rs/`) and data files in `$XDG_DATA_HOME/v2ray-rs/` (defaulting to `~/.local/share/v2ray-rs/`).

#### Scenario: First launch directory creation
- **WHEN** the app launches and storage directories do not exist
- **THEN** the system SHALL create them with appropriate permissions (0700)

#### Scenario: XDG override
- **WHEN** XDG_CONFIG_HOME is set to a custom path
- **THEN** the system SHALL use that path instead of the default

### Requirement: Settings persistence
The system SHALL serialize application settings to TOML format and store them in the config directory. The system SHALL deserialize settings on startup.

#### Scenario: Save and reload settings
- **WHEN** the user changes a setting and the app restarts
- **THEN** the changed setting SHALL be preserved

#### Scenario: Corrupt config handling
- **WHEN** the config file is corrupted or unparseable
- **THEN** the system SHALL fall back to defaults and warn the user

### Requirement: Data persistence
The system SHALL serialize subscriptions, proxy nodes, and routing rules to JSON format and store them in the data directory.

#### Scenario: Subscription data round-trip
- **WHEN** subscriptions are saved and reloaded
- **THEN** all subscription data including nodes and metadata SHALL be preserved

#### Scenario: Atomic writes
- **WHEN** data is saved to disk
- **THEN** the system SHALL use atomic write operations (write to temp file, then rename) to prevent data corruption on crash
