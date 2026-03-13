## MODIFIED Requirements

### Requirement: Tray context menu
The system SHALL display a context menu when the tray icon is activated.

#### Scenario: Menu when connected to manual node
- **WHEN** the user activates the tray icon while connected to a manual node
- **THEN** the menu status label shows `Connected` together with source `Manual` and the active node name

### Requirement: Tray tooltip status
The system SHALL include active connection details in the tray tooltip.

#### Scenario: Tooltip when connected to manual node
- **WHEN** the backend process is running successfully on a manual node
- **THEN** the tooltip shows connection state, source `Manual`, node, latency, backend, strategy, and connected-since timestamp
