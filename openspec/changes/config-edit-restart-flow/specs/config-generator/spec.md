## MODIFIED Requirements

### Requirement: Reactive config regeneration
The system SHALL automatically regenerate the config file when runtime configuration changes, with behavior depending on connection state.
- When the backend is stopped, runtime-relevant settings and routing-rule changes SHALL regenerate the config immediately.
- When the backend is starting or running, runtime-relevant settings and routing-rule changes SHALL be persisted but SHALL NOT replace the active runtime config until the user applies restart or reconnects.

#### Scenario: Disconnected routing change triggers regen
- **WHEN** the backend is stopped and the user changes a routing rule
- **THEN** the system regenerates the config immediately

#### Scenario: Connected DNS change waits for restart
- **WHEN** the backend is connected and the user changes DNS settings
- **THEN** the new settings are persisted, the active runtime config is marked as restart-required, and the running backend continues using the previous launched config
