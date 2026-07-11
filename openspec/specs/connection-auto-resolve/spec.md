## Purpose
Define how available nodes are resolved and used for connection attempts.
## Requirements
### Requirement: Build ordered connection candidates
The system SHALL build an ordered list of connection candidates from enabled subscription nodes and enabled manual nodes according to the selected strategy. The Lowest Latency strategy SHALL be configurable via the `use_real_delay_for_lowest_latency` app setting: when the setting is true and a node has a recorded `last_real_delay_ms`, the system SHALL rank by Real Delay; otherwise it SHALL fall back to `last_latency_ms` (TCP). Nodes with neither sample SHALL be placed last.

#### Scenario: List order
- **WHEN** the strategy is set to list order
- **THEN** candidates SHALL follow subscription order, then node order within each subscription

#### Scenario: Lowest latency by TCP (default)
- **WHEN** the strategy is set to lowest latency and `use_real_delay_for_lowest_latency` is false
- **THEN** candidates SHALL be ordered by ascending `last_latency_ms` with unknown values placed last

#### Scenario: Lowest latency by Real Delay
- **WHEN** the strategy is set to lowest latency and `use_real_delay_for_lowest_latency` is true
- **THEN** candidates SHALL be ordered by ascending `last_real_delay_ms` for nodes that have a sample, then by ascending `last_latency_ms` for nodes that lack a Real Delay sample, then by remaining order with unknown values placed last

#### Scenario: Random
- **WHEN** the strategy is set to random
- **THEN** candidates SHALL be shuffled for each connection attempt

#### Scenario: Last successful
- **WHEN** the strategy is set to last successful
- **THEN** the last successful node (if available and enabled) SHALL be first, followed by remaining candidates in list order

#### Scenario: Manual node included in candidate list
- **WHEN** a manual node is enabled
- **THEN** it appears in the candidate list alongside enabled subscription nodes

#### Scenario: Last successful manual node remains stable
- **WHEN** the last successful candidate was a manual node and another manual node is inserted or deleted
- **THEN** the stored last-success reference still points to the same manual node by ID

#### Scenario: Last successful subscription node remains stable
- **WHEN** the last successful candidate was a subscription node and subscriptions are reordered or refreshed
- **THEN** the stored last-success reference still points to the same subscription node by stable node ID

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

### Requirement: Strategy changes take effect on the next connection
A change to the auto-resolve strategy SHALL take effect at the next connection attempt. While a connection is active, the system SHALL NOT automatically disconnect to apply a strategy change; the running session continues under the strategy it was started with until the user explicitly applies the change or reconnects.

#### Scenario: Active session keeps its strategy
- **WHEN** the user changes the strategy while connected and does not apply the restart
- **THEN** the active session SHALL continue unchanged and its displayed connection metadata SHALL keep reporting the strategy it was started with

#### Scenario: Next connect uses the new strategy
- **WHEN** a new connection starts after a strategy change (explicit apply, manual reconnect, or a later connect)
- **THEN** candidate ordering SHALL follow the new strategy

