## Purpose

Polish the status bar and logs page with an ActionBar, icon+label connect button, and cross-fade empty state transitions.

## Requirements

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

#### Scenario: Starting state button is actionable
- **WHEN** the connection is starting, including an in-place crash respawn
- **THEN** the button shows the "Disconnect" label with `"destructive-action"` styling, stays sensitive, and invoking it cancels the attempt

### Requirement: Logs empty state uses Stack with crossfade
The logs page SHALL use `gtk::Stack` with crossfade transition to switch between the log view and the "Process Not Running" empty state.

#### Scenario: Transition from empty to logs
- **WHEN** the proxy process starts and logs begin appearing
- **THEN** the view crossfades from the StatusPage to the ScrolledWindow over 200ms

#### Scenario: Transition from logs to empty
- **WHEN** the proxy process stops
- **THEN** the view crossfades from the ScrolledWindow to the StatusPage over 200ms

### Requirement: Status bar shows connection health
While the connection is `Running`, the status bar SHALL reflect connection health. An unhealthy session SHALL show `Proxy not responding` as the status text, with the connection details followed by the last probe failure. A healthy session whose DNS through the proxy is marked failing SHALL show `Connected` with the details prefixed by `DNS via proxy failing`. A healthy session without the DNS mark SHALL show the existing connected text. The connect button SHALL keep its connected appearance in all three cases.

#### Scenario: Unhealthy status text
- **WHEN** the connection is `Running` and unhealthy with last failure `certificate has expired`
- **THEN** the status text SHALL be `Proxy not responding` and the details SHALL end with the failure reason

#### Scenario: DNS failing status text
- **WHEN** the connection is `Running`, healthy, and DNS through the proxy is marked failing
- **THEN** the status text SHALL be `Connected` and the details SHALL start with `DNS via proxy failing`

#### Scenario: Health recovers
- **WHEN** an unhealthy session becomes healthy with no DNS mark
- **THEN** the status bar SHALL show the same text as any connected session
