## MODIFIED Requirements

### Requirement: Reactive config regeneration
The system SHALL automatically regenerate the config file when subscription data, manual nodes, routing rules, or DNS settings change, with behavior depending on connection state.

#### Scenario: Subscription update triggers regen
- **WHEN** a subscription is updated with new nodes
- **THEN** the system SHALL regenerate the config within 1 second

#### Scenario: Disconnected manual node change triggers regen
- **WHEN** the backend is stopped and the user adds, edits, deletes, or toggles the enabled state of a manual node
- **THEN** the system SHALL regenerate the config immediately

#### Scenario: Connected manual node change waits for restart
- **WHEN** the backend is connected and the user adds, edits, deletes, or toggles the enabled state of a manual node
- **THEN** the change is persisted, but the active runtime config is not replaced until the user applies restart or reconnects later
