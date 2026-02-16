## MODIFIED Requirements

### Requirement: Status bar uses ActionBar widget
The connection status bar SHALL use `gtk::ActionBar` instead of a raw `gtk::Box` with manual toolbar styling.

#### Scenario: Status bar renders as ActionBar
- **WHEN** the main window is displayed
- **THEN** the bottom status bar is a `gtk::ActionBar` with primary status text and details packed start and connect button packed end

### Requirement: Connect button has icon and label
The connect/disconnect button SHALL display both a symbolic icon and a text label.

#### Scenario: Disconnected state button appearance
- **WHEN** the proxy is disconnected
- **THEN** the button shows `network-wireless-symbolic` icon with "Connect" label and `"suggested-action"` styling

#### Scenario: Connected state button appearance
- **WHEN** the proxy is connected
- **THEN** the button shows `network-wireless-disabled-symbolic` icon with "Disconnect" label and `"destructive-action"` styling
