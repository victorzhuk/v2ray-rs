## Purpose

Keep node latency fresh without disturbing active connections by using direct TCP probes, scheduled background refreshes, and startup hydration of persisted samples.

## Requirements

### Requirement: Direct TCP latency refresh while connected
Outside TUN sessions, manual and scheduled TCP latency probes SHALL leave the running backend unchanged. Each run SHALL bound simultaneous lookups and connections independently of the node count. During active TUN sessions, the UI SHALL disable direct TCP testing and explain that testing requires disconnecting TUN. Real Delay SHALL remain a separate manual action.

#### Scenario: Manual latency test while connected
- **WHEN** the user requests TCP latency measurements while connected without TUN
- **THEN** the system SHALL record direct measurements without stopping or restarting the backend

#### Scenario: Manual latency test unavailable under TUN
- **WHEN** a TUN connection is active
- **THEN** the disabled TCP action SHALL explain that TUN must disconnect before testing
- **AND** the hint SHALL NOT direct the user to run Real Delay during the active session

#### Scenario: Real Delay button is separate
- **WHEN** the user chooses Test Latency outside TUN
- **THEN** the system SHALL use TCP probes and leave Real Delay available as a separate action
- **AND** startup and scheduled refreshes SHALL NOT invoke Real Delay

#### Scenario: More nodes than concurrent probe slots
- **WHEN** the number of eligible nodes exceeds the concurrency limit
- **THEN** remaining nodes SHALL wait for available slots and each result SHALL identify its original node

### Requirement: Session-local scheduled latency refresh
The system SHALL run a session-local background latency refresh every 10 minutes for enabled nodes in enabled subscriptions while the app is running, except while the active connection has TUN enabled, when the tick SHALL be skipped entirely. The scheduled refresh SHALL use direct TCP probes only; it SHALL NOT invoke Real Delay probes, which are more expensive and remain on-demand.

#### Scenario: Scheduled refresh tick
- **WHEN** the 10-minute background timer fires and no TUN connection is active
- **THEN** the system refreshes TCP latency for enabled nodes that are not already under test and persists the completed samples for future connection ordering

#### Scenario: Scheduled refresh paused under TUN
- **WHEN** the 10-minute background timer fires while a TUN connection is active
- **THEN** the system SHALL perform no TCP probes and SHALL resume refreshing on the first tick after the TUN session ends

#### Scenario: Scheduled refresh does not spawn ephemeral backends
- **WHEN** the 10-minute background timer fires
- **THEN** the system SHALL NOT spawn any ephemeral backend instance for Real Delay testing

### Requirement: Startup latency hydration
On startup, the system SHALL hydrate `last_latency_ms` for all subscription nodes from the persisted `latency_snapshot.json`, so the UI displays known TCP latency values immediately rather than waiting for the first scheduled refresh. The system SHALL also hydrate `last_real_delay_ms` from the same snapshot file when present, so the UI displays the previously recorded Real Delay value alongside the TCP value without requiring a fresh probe.

#### Scenario: Startup shows previous TCP latency
- **WHEN** the app launches and a persisted latency snapshot exists with a TCP sample
- **THEN** each subscription node row that has a recorded sample displays its TCP latency value without requiring a manual test or waiting 10 minutes

#### Scenario: Startup shows previous Real Delay when present
- **WHEN** the app launches and a persisted latency snapshot exists with a Real Delay sample
- **THEN** the corresponding node row displays its Real Delay value alongside the TCP value
