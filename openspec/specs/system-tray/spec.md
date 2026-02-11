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
- **THEN** the menu SHALL show: "Connect", separator, profile info, "Open Main Window", "Quit"

#### Scenario: Menu when connected
- **WHEN** the user activates the tray icon while connected
- **THEN** the menu SHALL show: "Disconnect", separator, active profile name, "Open Main Window", "Quit"

### Requirement: Quick connect/disconnect
The system SHALL allow connecting and disconnecting via the tray menu.

#### Scenario: Connect from tray
- **WHEN** the user clicks "Connect" in the tray menu
- **THEN** the system SHALL start the backend process with the current config

#### Scenario: Disconnect from tray
- **WHEN** the user clicks "Disconnect" in the tray menu
- **THEN** the system SHALL gracefully stop the backend process

### Requirement: Tray notifications
The system SHALL display desktop notifications for connection state changes.

#### Scenario: Connection established notification
- **WHEN** the backend process starts successfully
- **THEN** the system SHALL display a notification "Proxy connected"

#### Scenario: Connection lost notification
- **WHEN** the backend process exits unexpectedly
- **THEN** the system SHALL display a notification "Proxy disconnected unexpectedly"

#### Scenario: Notifications configurable
- **WHEN** the user disables notifications in settings
- **THEN** the system SHALL NOT display any tray notifications

### Requirement: Minimize to tray
The system SHALL support minimizing the main window to the system tray instead of closing.

#### Scenario: Close button minimizes
- **WHEN** the user clicks the window close button and minimize-to-tray is enabled
- **THEN** the main window SHALL be hidden and the app SHALL continue running in the tray

#### Scenario: Restore from tray
- **WHEN** the user clicks "Open Main Window" in the tray menu
- **THEN** the main window SHALL be shown and focused
