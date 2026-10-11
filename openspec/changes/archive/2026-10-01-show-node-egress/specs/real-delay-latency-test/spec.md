## ADDED Requirements

### Requirement: Egress address is detected during probe sessions
A Real Delay probe session SHALL additionally determine, for each tested node, the public egress address and location code observed by that node, by issuing one HTTP request through a per-node loopback inbound routed exclusively to that node's outbound and parsing the reply of a built-in IP-echo endpoint. The egress result SHALL be reported alongside the node's delay result, SHALL be recorded as unknown when the request fails, and SHALL NOT affect the node's delay measurement.

#### Scenario: Egress parsed from the echo endpoint
- **WHEN** a probe session tests a node and the egress request through that node returns a 200 reply containing `ip=2001:db8::1` and `loc=AT`
- **THEN** the node's probe result SHALL carry egress address `2001:db8::1` and location `AT`

#### Scenario: Failed egress request leaves the sample unknown
- **WHEN** the egress request for a node times out or returns a non-200 or unparseable reply
- **THEN** that node's egress SHALL be reported as unknown and its delay result SHALL be unaffected

#### Scenario: One inbound per tested node bound to loopback
- **WHEN** the probe config is generated for N tested nodes
- **THEN** it SHALL contain N HTTP inbounds bound to `127.0.0.1` on distinct OS-assigned ports, each routed by inbound tag to exactly its node's outbound tag, and no inbound SHALL be reachable from a non-loopback interface

#### Scenario: No extra backend instance for egress
- **WHEN** a probe session runs with egress detection
- **THEN** the session SHALL use the same single ephemeral backend instance as the delay probes and SHALL NOT spawn additional processes

### Requirement: Per-node egress persistence
The system SHALL persist the most recent egress sample per subscription node and per manual node in the latency snapshot file alongside the delay samples, keyed by stable node identity. A missing or stale sample SHALL be representable as unknown without conflating it with a successful result.

#### Scenario: Sample is written after a successful probe
- **WHEN** a probe session records egress address and location for a node
- **THEN** the node's egress sample SHALL be persisted atomically to the latency snapshot file with the checked-at timestamp

#### Scenario: Failed detection keeps the previous sample
- **WHEN** a probe session cannot determine a node's egress and the node has a previously persisted sample
- **THEN** the previous sample SHALL remain intact and SHALL NOT be overwritten as a successful result

#### Scenario: Startup hydration of egress
- **WHEN** the app launches and the latency snapshot contains egress samples
- **THEN** each affected node SHALL expose its last recorded egress without requiring a fresh probe
