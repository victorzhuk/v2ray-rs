## MODIFIED Requirements

### Requirement: Application window structure
The system SHALL display a main window with a header bar, an upper-pane source switcher for `Subscriptions` and `Nodes`, a logs pane, and a connection status bar.

#### Scenario: Nodes section visible
- **WHEN** the main window is displayed
- **THEN** the upper pane lets the user switch between subscription management and manual-node management without opening Preferences

### Requirement: Connection status bar
The system SHALL display a persistent status bar showing current connection state with a connect or disconnect button and active connection details for both subscription and manual nodes.

#### Scenario: Status bar when connected to manual node
- **WHEN** the active connection comes from a manual node
- **THEN** the status bar shows `Manual` as the source label together with the node name, latency, backend, strategy, and connected-since timestamp
