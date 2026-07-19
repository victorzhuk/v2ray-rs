## Purpose

Keep node latency fresh without disturbing active connections by using direct TCP probes, scheduled background refreshes, and startup hydration of persisted samples.
## Requirements
### Requirement: Direct TCP latency refresh while connected
The system SHALL refresh node latency with direct TCP probes without interrupting or modifying the currently running backend process — except while the active connection has TUN enabled, when raw TCP probing SHALL be unavailable: probes captured by the tunnel measure the proxied path, not direct latency, and connect-then-close probes crash affected Xray-core versions (26.1.13–26.6.22, upstream #6364). The "Test Latency" action in the subscriptions UI SHALL continue to use the TCP probe outside TUN sessions; during an active TUN session it SHALL be insensitive with a hint pointing at "Test Real Delay". The "Test Real Delay" action SHALL remain a separate, opt-in operation handled by the `real-delay-latency-test` capability and SHALL NOT be triggered from scheduled or startup paths.

#### Scenario: Manual latency test while connected
- **WHEN** the user triggers a TCP latency test while the proxy is connected without TUN
- **THEN** the system performs direct TCP probes and records the results without stopping or restarting the backend

#### Scenario: Manual latency test unavailable under TUN
- **WHEN** a TUN connection is active
- **THEN** the "Test Latency" action SHALL be insensitive with a hint that direct probing is unavailable under TUN and that "Test Real Delay" measures through the tunnel

#### Scenario: Real Delay button is separate
- **WHEN** the user looks at the subscriptions UI
- **THEN** the existing "Test Latency" button SHALL still trigger TCP probes outside TUN sessions, and a distinct "Test Real Delay" action SHALL be available for invoking the end-to-end probe defined by `real-delay-latency-test`

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

