# Spec: main-window

## ADDED Requirements

### Requirement: Application window structure
The system SHALL display a main window with a header bar, a vertical split between subscriptions and logs, and a connection status bar.

#### Scenario: Window layout
- **WHEN** the main window is displayed
- **THEN** it SHALL contain a header bar with app title and hamburger menu, a `gtk::Paned` vertically splitting the subscriptions page (top) and logs page (bottom), and a bottom `gtk::ActionBar` showing connection state. Routing and Settings are accessible via the hamburger menu → Preferences dialog.

### Requirement: Subscriptions page
The system SHALL provide a page for managing proxy subscriptions.

#### Scenario: View subscriptions
- **WHEN** the user navigates to the Subscriptions tab
- **THEN** the system SHALL display a list of all subscriptions with name, URL, node count, and last updated time

#### Scenario: Add subscription
- **WHEN** the user clicks "Add Subscription"
- **THEN** the system SHALL show a dialog to enter a name and URL, then import and parse the subscription

#### Scenario: View subscription nodes
- **WHEN** the user expands a subscription
- **THEN** the system SHALL show all nodes with name, address, protocol, and enable/disable toggle

#### Scenario: Remove subscription
- **WHEN** the user deletes a subscription
- **THEN** the system SHALL remove it and all its nodes after confirmation

### Requirement: Routing rules and Settings
The system SHALL provide routing rule management and settings via the hamburger menu → Preferences dialog (not a main-window tab).

#### Scenario: Open Preferences
- **WHEN** the user opens the hamburger menu and selects "Preferences"
- **THEN** the system SHALL show an `adw::PreferencesDialog` with pages: System, Network, Routing, DNS

### Requirement: Logs page
The system SHALL provide a page for viewing backend process logs.

#### Scenario: Live log display
- **WHEN** the backend process is running and the Logs tab is active
- **THEN** the system SHALL display log lines in real-time, auto-scrolling to the latest entry

#### Scenario: Log when stopped
- **WHEN** no backend process is running
- **THEN** the Logs page SHALL show the last session's logs (if any) with a "Process not running" indicator

### Requirement: Connection status bar
The system SHALL display a persistent status bar showing current connection state with a connect/disconnect button and active connection details.

#### Scenario: Status bar when connected
- **WHEN** the backend process is running
- **THEN** the status bar SHALL show "Connected" with the active subscription name, node name, latency, backend, strategy, connected-since timestamp, and a "Disconnect" button

#### Scenario: Status bar when disconnected
- **WHEN** no backend process is running
- **THEN** the status bar SHALL show "Disconnected" with a "Connect" button and placeholders indicating no active node
