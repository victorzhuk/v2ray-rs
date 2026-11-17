## MODIFIED Requirements

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
