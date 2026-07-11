# Spec: main-window

## Purpose

Define the primary application window layout with subscriptions/nodes management, logs, status bar, and access to routing and settings.

## Requirements

### Requirement: Application window structure
The system SHALL display a main window with a header bar, an upper-pane source switcher for `Subscriptions` and `Nodes`, a logs pane, and a connection status bar.

#### Scenario: Window layout
- **WHEN** the main window is displayed
- **THEN** it SHALL contain a header bar with app title and hamburger menu, a `gtk::Paned` vertically splitting the subscriptions page (top) and logs page (bottom), and a bottom `gtk::ActionBar` showing connection state. Routing and Settings are accessible via the hamburger menu → Preferences dialog.

#### Scenario: Nodes section visible
- **WHEN** the main window is displayed
- **THEN** the upper pane lets the user switch between subscription management and manual-node management without opening Preferences

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
The system SHALL keep the current-session log buffer visible even while the backend is stopped.

#### Scenario: Log view while stopped
- **WHEN** no backend process is running
- **THEN** the Logs page SHALL continue showing the most recent in-memory logs together with a "Process not running" indicator

### Requirement: Connection status bar
The system SHALL display a persistent status bar showing current connection state with a connect or disconnect button and active connection details for both subscription and manual nodes.

#### Scenario: Status bar when connected
- **WHEN** the backend process is running
- **THEN** the status bar SHALL show "Connected" with the active subscription name, node name, latency, backend, strategy, connected-since timestamp, and a "Disconnect" button

#### Scenario: Status bar when disconnected
- **WHEN** no backend process is running
- **THEN** the status bar SHALL show "Disconnected" with a "Connect" button and placeholders indicating no active node

#### Scenario: Status bar when connected to manual node
- **WHEN** the active connection comes from a manual node
- **THEN** the status bar shows `Manual` as the source label together with the node name, latency, backend, strategy, and connected-since timestamp
