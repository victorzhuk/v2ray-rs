# Spec: System Tray (delta)

## ADDED Requirements

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
- **THEN** the menu SHALL show: "Connect", separator, status label ("Status: Disconnected", disabled), separator, "Open Main Window", "Quit"

#### Scenario: Menu when connected
- **WHEN** the user activates the tray icon while connected
- **THEN** the menu SHALL show: "Disconnect", separator, status label ("Status: Connected (node name)", disabled), separator, "Open Main Window", "Quit"

#### Scenario: Menu when connected to manual node
- **WHEN** the user activates the tray icon while connected to a manual node
- **THEN** the menu status label shows `Connected` together with source `Manual` and the active node name

#### Scenario: Connect/Disconnect disabled during transitions
- **WHEN** the state is Starting or Stopping
- **THEN** the Connect/Disconnect menu item SHALL be disabled (not clickable)

### Requirement: Tray tooltip status
The system SHALL include active connection details in the tray tooltip.

#### Scenario: Tooltip when connected
- **WHEN** the backend process is running successfully
- **THEN** the tooltip SHALL show connection state, subscription, node, latency, backend, strategy, and connected-since timestamp

#### Scenario: Tooltip when connected to manual node
- **WHEN** the backend process is running successfully on a manual node
- **THEN** the tooltip shows connection state, source `Manual`, node, latency, backend, strategy, and connected-since timestamp

#### Scenario: Tooltip when disconnected
- **WHEN** the backend process is not running
- **THEN** the tooltip SHALL show "Disconnected" with no active node details

### Requirement: Quick connect/disconnect
The system SHALL allow connecting and disconnecting via the tray menu.

#### Scenario: Connect from tray
- **WHEN** the user clicks "Connect" in the tray menu
- **THEN** the system SHALL start the backend process with the current config

#### Scenario: Disconnect from tray
- **WHEN** the user clicks "Disconnect" in the tray menu
- **THEN** the system SHALL gracefully stop the backend process

### Requirement: Minimize to tray
The system SHALL support minimizing the main window to the system tray instead of closing.

#### Scenario: Close button minimizes
- **WHEN** the user clicks the window close button and minimize-to-tray is enabled
- **THEN** the main window SHALL be hidden and the app SHALL continue running in the tray

#### Scenario: Restore from tray
- **WHEN** the user clicks "Open Main Window" in the tray menu
- **THEN** the main window SHALL be shown and focused

### Requirement: Tray notifications
The system SHALL support optional desktop notifications for tray-visible connection state changes.

#### Scenario: Notifications enabled
- **WHEN** notifications are enabled and the connection state changes to Running or Error
- **THEN** the tray integration SHALL emit a desktop notification describing the state change