# Spec: main-window

## ADDED Requirements

### Requirement: Application window structure
The system SHALL display a main window with a header bar, tab navigation, and a connection status bar.

#### Scenario: Window layout
- **WHEN** the main window is displayed
- **THEN** it SHALL contain a header bar with app title, a tab switcher (Subscriptions, Routing, Logs, Settings), the active page content, and a bottom status bar showing connection state

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

### Requirement: Routing rules page
The system SHALL provide a page for managing routing rules.

#### Scenario: View rules
- **WHEN** the user navigates to the Routing tab
- **THEN** the system SHALL display an ordered list of routing rules with match condition, action, and drag handles for reordering

#### Scenario: Add rule
- **WHEN** the user clicks "Add Rule"
- **THEN** the system SHALL show a dialog to select rule type (GeoIP, GeoSite, Domain, IP), enter match value, and select action

#### Scenario: Drag-and-drop reordering
- **WHEN** the user drags a rule to a new position
- **THEN** the rule order SHALL update and config SHALL be regenerated

### Requirement: Logs page
The system SHALL provide a page for viewing backend process logs.

#### Scenario: Live log display
- **WHEN** the backend process is running and the Logs tab is active
- **THEN** the system SHALL display log lines in real-time, auto-scrolling to the latest entry

#### Scenario: Log when stopped
- **WHEN** no backend process is running
- **THEN** the Logs page SHALL show the last session's logs (if any) with a "Process not running" indicator

### Requirement: Settings page
The system SHALL provide a page for app configuration.

#### Scenario: Backend selection
- **WHEN** the user navigates to Settings
- **THEN** the system SHALL show detected backends with radio buttons to select the active one

#### Scenario: Proxy port configuration
- **WHEN** the user changes the SOCKS5 or HTTP port
- **THEN** the system SHALL save the setting and trigger config regeneration

#### Scenario: Language selection
- **WHEN** the user changes the language
- **THEN** the UI SHALL switch to the selected language

### Requirement: Connection status bar
The system SHALL display a persistent status bar showing current connection state with a connect/disconnect button.

#### Scenario: Status bar when connected
- **WHEN** the backend process is running
- **THEN** the status bar SHALL show "Connected" with the active profile name and a "Disconnect" button

#### Scenario: Status bar when disconnected
- **WHEN** no backend process is running
- **THEN** the status bar SHALL show "Disconnected" with a "Connect" button
