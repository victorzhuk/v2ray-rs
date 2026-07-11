## Purpose

Keep node latency fresh without disturbing active connections by using direct TCP probes, scheduled background refreshes, and startup hydration of persisted samples.

## Requirements

### Requirement: Direct TCP latency refresh while connected
The system SHALL refresh node latency with direct TCP probes without interrupting or modifying the currently running backend process. The "Test Latency" action in the subscriptions UI SHALL continue to use the TCP probe; the new "Test Real Delay" action SHALL be a separate, opt-in operation handled by the `real-delay-latency-test` capability and SHALL NOT be triggered from scheduled or startup paths.

#### Scenario: Manual latency test while connected
- **WHEN** the user triggers a TCP latency test while the proxy is connected
- **THEN** the system performs direct TCP probes and records the results without stopping or restarting the backend

#### Scenario: Real Delay button is separate
- **WHEN** the user looks at the subscriptions UI
- **THEN** the existing "Test Latency" button SHALL still trigger TCP probes, and a distinct "Test Real Delay" action SHALL be available for invoking the end-to-end probe defined by `real-delay-latency-test`

### Requirement: Session-local scheduled latency refresh
The system SHALL run a session-local background latency refresh every 10 minutes for enabled nodes in enabled subscriptions while the app is running. The 10-minute interval balances freshness with network overhead for typical VPN usage patterns. The scheduled refresh SHALL use direct TCP probes only; it SHALL NOT invoke Real Delay probes, which are more expensive and remain on-demand.

#### Scenario: Scheduled refresh tick
- **WHEN** the 10-minute background timer fires
- **THEN** the system refreshes TCP latency for enabled nodes that are not already under test and persists the completed samples for future connection ordering

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
