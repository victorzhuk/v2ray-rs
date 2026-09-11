## MODIFIED Requirements

### Requirement: Direct connection to a chosen node
The system SHALL let the user connect directly to a specific enabled node, using that node as the only connection candidate for the attempt. The action SHALL NOT change the configured auto-resolve strategy, and subsequent ordinary connects SHALL use the configured strategy unchanged. Reconnects the system starts on the user's behalf for that session — applying pending changes and automatic reconnects after a failure — SHALL target the same node as the sole candidate.

#### Scenario: Connect to a specific node
- **WHEN** the user invokes Connect on a specific enabled node
- **THEN** the system SHALL attempt the connection with that node as the sole candidate, without falling back to other nodes on failure

#### Scenario: Direct connect while already connected
- **WHEN** the user invokes Connect on a node while a connection is active
- **THEN** the system SHALL stop the current session and connect to the chosen node

#### Scenario: Direct connect failure surfaces immediately
- **WHEN** the directly chosen node fails to connect
- **THEN** the system SHALL surface the error without trying any other candidate

#### Scenario: Direct connect updates last-success metadata
- **WHEN** a direct connection succeeds
- **THEN** the system SHALL record it as the last successful node, the same as any other successful connection

#### Scenario: Automatic reconnect keeps the chosen node
- **WHEN** a session started by a direct connection gives up after crashes and an automatic reconnect runs
- **THEN** the reconnect SHALL target the directly chosen node as the sole candidate

## ADDED Requirements

### Requirement: Last-success metadata is owned by connection outcomes
The last-success record SHALL change only when a connection succeeds. Saving settings from the preferences dialog SHALL NOT modify it.

#### Scenario: Preferences opened before a connect
- **WHEN** the preferences dialog is open, a connection to node B succeeds, and the user then changes any preference
- **THEN** the persisted last-success record SHALL still name node B

### Requirement: Automatic reconnect yields to the user
A pending automatic reconnect SHALL be cancelled by any user-initiated Connect, direct connect, or Disconnect, so user retries neither trigger an extra immediate attempt nor consume the automatic-reconnect budget.

#### Scenario: Manual connect during the reconnect delay
- **WHEN** an automatic reconnect is scheduled and the user invokes Connect before it fires
- **THEN** the scheduled reconnect SHALL NOT fire, and only the user's attempt SHALL run

#### Scenario: Disconnect during the reconnect delay
- **WHEN** an automatic reconnect is scheduled and the user invokes Disconnect
- **THEN** the scheduled reconnect SHALL NOT fire
