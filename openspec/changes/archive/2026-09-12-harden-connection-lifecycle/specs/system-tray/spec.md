## MODIFIED Requirements

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

#### Scenario: Disconnect enabled while starting
- **WHEN** the state is Starting
- **THEN** the menu SHALL show an enabled "Disconnect" item that cancels the connection attempt

#### Scenario: Connect/Disconnect disabled during transitions
- **WHEN** the state is Stopping
- **THEN** the Connect/Disconnect menu item SHALL be disabled (not clickable)
