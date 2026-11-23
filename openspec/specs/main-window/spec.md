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

### Requirement: Status bar distinguishes restarts from connects
While a connection is starting, the status bar SHALL distinguish why: an in-place respawn after the backend exited unexpectedly SHALL show "Restarting after crash", and an attempt started by auto-reconnect SHALL show "Reconnecting (n/3)" with the current attempt number. Other starts SHALL keep showing "Connecting…".

#### Scenario: Crash respawn
- **WHEN** a connected backend exits unexpectedly and the system respawns it
- **THEN** the status bar SHALL show "Restarting after crash" until the backend is running again or the connection ends

#### Scenario: Auto-reconnect attempt
- **WHEN** the second scheduled auto-reconnect starts a connection
- **THEN** the status bar SHALL show "Reconnecting (2/3)"

#### Scenario: User connect
- **WHEN** the user clicks Connect
- **THEN** the status bar SHALL show "Connecting…"

### Requirement: Application localization applies message catalogs without altering numeric locale
At startup, the UI SHALL initialize `gettext` for `LC_MESSAGES` using the persisted `Language` setting without overriding `LC_NUMERIC` or `LC_ALL`, and SHALL look up primary window, status bar, Preferences, and dialog strings through the translation catalog. Changing `Language` in Preferences SHALL persist the choice and inform the user that restarting the application completes full UI relabeling.

#### Scenario: Russian language selection translates UI without comma decimal separators
- **WHEN** `settings.language` is `Russian` (`ru_RU`) at application startup
- **THEN** primary window and Preferences labels SHALL resolve from the `ru_RU` message catalog while floating-point formatting under `LC_NUMERIC` continues using `.` decimal separators

#### Scenario: Language change surfaces restart notice
- **WHEN** the user changes the `Language` dropdown in System Preferences while the app is running
- **THEN** the new `Language` SHALL be persisted and the UI SHALL display a non-blocking notice stating that an application restart is required to relabel open views

### Requirement: CleanupPending exposes a persistent retry control and disables Connect
While `cleanup_pending` is true, the main window SHALL disable the Connect button, suppress duplicate generic error toasts from `apply_state`, and display a persistent banner offering "Retry cleanup" until cleanup succeeds.

#### Scenario: Retry cleanup remains accessible after toast timeout
- **WHEN** `AppMsg::CleanupPending` arrives and more than 5 seconds elapse
- **THEN** a persistent banner SHALL still offer "Retry cleanup", Connect SHALL be insensitive, and only one error/cleanup notification SHALL have appeared

### Requirement: Broken settings reset flows through WorkspaceStore
`AppMsg::ResetBrokenSettings` SHALL reset `settings.toml` and update the cached `AppSettings` snapshot through `WorkspaceStore::reset_settings()` and SHALL resync `geodata_service` and `subscriptions_page` whether or not the file existed on disk.

#### Scenario: Store cache reflects reset settings immediately
- **WHEN** the user confirms resetting a broken `settings.toml`
- **THEN** `WorkspaceStore::load_settings()` SHALL immediately return `AppSettings::default()` and dependent UI/services SHALL resync
