## ADDED Requirements

### Requirement: Asynchronous node latency indicators
Node rows in the subscriptions UI SHALL display their most recent latency values and update when manual or scheduled latency refreshes complete.

#### Scenario: Manual latency result updates a row
- **WHEN** a user-triggered latency test finishes for a subscription node
- **THEN** the matching node row updates its latency text and styling without disconnecting the backend

#### Scenario: Scheduled latency result updates a row
- **WHEN** the 10-minute background refresh finishes for an enabled node
- **THEN** the matching node row updates its latency indicator and keeps the current connection state unchanged

#### Scenario: Latency color coding
- **WHEN** a node's latency is below 200ms THEN the label uses success styling (green)
- **WHEN** a node's latency is between 200ms and 499ms THEN the label uses warning styling (yellow)
- **WHEN** a node's latency is 500ms or above THEN the label uses error styling (red)
- **WHEN** a node has no latency data THEN no latency label is shown

#### Scenario: Startup latency display
- **WHEN** the app starts with a persisted latency snapshot
- **THEN** node rows immediately display their last recorded latency values
