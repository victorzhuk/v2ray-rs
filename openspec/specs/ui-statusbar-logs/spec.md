## ADDED Requirements

### Requirement: Status bar uses ActionBar widget
The connection status bar SHALL use `gtk::ActionBar` instead of a raw `gtk::Box` with manual toolbar styling.

#### Scenario: Status bar renders as ActionBar
- **WHEN** the main window is displayed
- **THEN** the bottom status bar is a `gtk::ActionBar` with status text packed start and connect button packed end

### Requirement: Connect button has icon and label
The connect/disconnect button SHALL display both a symbolic icon and a text label.

#### Scenario: Disconnected state button appearance
- **WHEN** the proxy is disconnected
- **THEN** the button shows `network-wireless-symbolic` icon with "Connect" label and `"suggested-action"` styling

#### Scenario: Connected state button appearance
- **WHEN** the proxy is connected
- **THEN** the button shows `network-wireless-disabled-symbolic` icon with "Disconnect" label and `"destructive-action"` styling

### Requirement: Logs empty state uses Stack with crossfade
The logs page SHALL use `gtk::Stack` with crossfade transition to switch between the log view and the "Process Not Running" empty state.

#### Scenario: Transition from empty to logs
- **WHEN** the proxy process starts and logs begin appearing
- **THEN** the view crossfades from the StatusPage to the ScrolledWindow over 200ms

#### Scenario: Transition from logs to empty
- **WHEN** the proxy process stops
- **THEN** the view crossfades from the ScrolledWindow to the StatusPage over 200ms
