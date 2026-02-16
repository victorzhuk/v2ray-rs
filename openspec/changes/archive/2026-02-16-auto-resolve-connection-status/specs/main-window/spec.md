## MODIFIED Requirements

### Requirement: Connection status bar
The system SHALL display a persistent status bar showing current connection state with a connect/disconnect button and active connection details.

#### Scenario: Status bar when connected
- **WHEN** the backend process is running
- **THEN** the status bar SHALL show "Connected" with the active subscription name, node name, latency, backend, strategy, connected-since timestamp, and a "Disconnect" button

#### Scenario: Status bar when disconnected
- **WHEN** no backend process is running
- **THEN** the status bar SHALL show "Disconnected" with a "Connect" button and placeholders indicating no active node
