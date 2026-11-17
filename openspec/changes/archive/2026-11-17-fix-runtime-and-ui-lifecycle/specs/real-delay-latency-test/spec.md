# Spec Delta

## ADDED Requirements

### Requirement: Ephemeral probes pause during active TUN and bound batch resources
Manual and background `Test Real Delay` and `Check Availability` runs SHALL be blocked with an explanatory toast while a TUN session is active, `REAL_DELAY_LOCK` SHALL remain held until `ProbeRunner` finishes stopping, probe runs SHALL partition nodes into batches of at most 64 loopback inbounds with caller deadlines scaled by batch count, and Clash API responses SHALL enforce a bounded body read.

#### Scenario: Real Delay and Availability actions blocked during active TUN
- **WHEN** a TUN session is active (`Starting` or `Running`) and the user invokes `Test Real Delay` or `Check Availability`
- **THEN** the system SHALL refuse to start an unmarked ephemeral probe backend and SHALL display a toast explaining that probes require disconnecting TUN first

#### Scenario: Large subscription probes run in bounded batches
- **WHEN** the user runs `Test Real Delay` on 150 enabled nodes while non-TUN
- **THEN** the session SHALL execute in batches of at most 64 loopback inbounds without exhausting file descriptors or timing out after a single-batch deadline

#### Scenario: A second probe starts during teardown
- **WHEN** a probe is cancelled and another probe requests the shared lock while the first backend is stopping
- **THEN** the second probe SHALL wait until the first backend finishes stopping before it starts its own backend
