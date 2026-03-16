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

### Requirement: Restore persisted runtime configuration
The system SHALL be able to restore persisted settings and routing rules to the last launched runtime snapshot when the user discards connected-state changes.

#### Scenario: Discard connected edits
- **WHEN** the user selects "Discard" from the restart-required banner
- **THEN** the system overwrites the current persisted settings and routing rules with the active runtime snapshot and clears the pending divergence state

### Requirement: Custom nodes persistence
The system SHALL persist manual proxy nodes in `custom_nodes.json` under the app data directory using atomic writes.

#### Scenario: Save and reload manual nodes
- **WHEN** manual nodes are saved and the app restarts
- **THEN** the nodes round-trip with their IDs, enabled state, and protocol data intact

#### Scenario: Corrupt custom nodes file
- **WHEN** `custom_nodes.json` is unreadable or invalid
- **THEN** the system falls back to an empty manual-node list and reports the problem without preventing startup

### Requirement: Restore launched manual nodes on discard
The system SHALL restore the launched manual-node set when the user discards connected-state manual-node changes.

#### Scenario: Discard connected manual-node edits
- **WHEN** the backend is connected, the user changes manual nodes, and then selects `Discard`
- **THEN** the system overwrites the current persisted manual-node set with the last launched manual-node snapshot and clears the pending divergence state
