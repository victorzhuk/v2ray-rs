## ADDED Requirements

### Requirement: Direct connection to a chosen node
The system SHALL let the user connect directly to a specific enabled node, using that node as the only connection candidate for the attempt. The action SHALL NOT change the configured auto-resolve strategy, and subsequent ordinary connects SHALL use the configured strategy unchanged.

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
