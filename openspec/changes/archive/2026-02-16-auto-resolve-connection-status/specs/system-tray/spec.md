## MODIFIED Requirements

### Requirement: Tray icon display
The system SHALL display a system tray icon that reflects the current proxy connection state.

#### Scenario: Disconnected state
- **WHEN** no backend process is running
- **THEN** the tray icon SHALL display a gray/inactive icon

#### Scenario: Connected state
- **WHEN** the backend process is running successfully
- **THEN** the tray icon SHALL display a colored/active icon

#### Scenario: Error state
- **WHEN** the backend process has crashed or is in error state
- **THEN** the tray icon SHALL display a red/error icon

### Requirement: Tray context menu
The system SHALL display a context menu when the tray icon is activated.

#### Scenario: Menu when disconnected
- **WHEN** the user activates the tray icon while disconnected
- **THEN** the menu SHALL show: "Connect", separator, profile info, "Open Main Window", "Quit"

#### Scenario: Menu when connected
- **WHEN** the user activates the tray icon while connected
- **THEN** the menu SHALL show: "Disconnect", separator, active profile name, "Open Main Window", "Quit"

### Requirement: Tray tooltip status
The system SHALL include active connection details in the tray tooltip.

#### Scenario: Tooltip when connected
- **WHEN** the backend process is running successfully
- **THEN** the tooltip SHALL show connection state, subscription, node, latency, backend, strategy, and connected-since timestamp

#### Scenario: Tooltip when disconnected
- **WHEN** the backend process is not running
- **THEN** the tooltip SHALL show "Disconnected" with no active node details
