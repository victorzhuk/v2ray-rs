## ADDED Requirements

### Requirement: Direct TCP latency refresh while connected
The system SHALL refresh node latency with direct TCP probes without interrupting or modifying the currently running backend process.

#### Scenario: Manual latency test while connected
- **WHEN** the user triggers a latency test while the proxy is connected
- **THEN** the system performs direct TCP probes and records the results without stopping or restarting the backend

### Requirement: Session-local scheduled latency refresh
The system SHALL run a session-local background latency refresh every 10 minutes for enabled nodes in enabled subscriptions while the app is running.

#### Scenario: Scheduled refresh tick
- **WHEN** the 10-minute background timer fires
- **THEN** the system refreshes latency for enabled nodes that are not already under test and persists the completed samples for future connection ordering

### Requirement: Startup latency hydration
On startup, the system SHALL hydrate `last_latency_ms` for all subscription nodes from the persisted `latency_snapshot.json`, so the UI displays known latency values immediately.

#### Scenario: Startup shows previous latency
- **WHEN** the app launches and a persisted latency snapshot exists
- **THEN** each subscription node row that has a recorded sample displays its latency value without requiring a manual test or waiting 10 minutes
