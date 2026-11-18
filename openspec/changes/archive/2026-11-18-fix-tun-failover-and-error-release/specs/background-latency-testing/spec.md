# Spec Delta

## ADDED Requirements

### Requirement: Node probes stay suppressed whenever TUN owns host routing
Manual TCP latency tests, scheduled 10-minute background latency ticks, Real Delay probes, and service availability checks SHALL treat TUN as active whenever a TUN-configured session is `Starting`, `Running`, or `Stopping`, while TUN route teardown is in flight, or while a fail-closed TUN session remains parked in `Error` awaiting reconnect or recovery.

#### Scenario: Background latency tick skipped during fail-closed TUN reconnect backoff
- **WHEN** a strict TUN session is in `Error` with fail-closed routes still installed while automatic reconnect backoff is pending, and the 10-minute background latency timer fires
- **THEN** the system SHALL skip TCP latency probes and preserve existing node latency samples for candidate planning

#### Scenario: Manual probes blocked while TUN is starting or tearing down
- **WHEN** a TUN session is in `Starting` or `Stopping` or TUN route release is in flight, and the user invokes Test Latency, Test Real Delay, or Check Availability
- **THEN** the probe SHALL NOT run against host routing and persisted latency, Real Delay, egress, and availability records SHALL remain unchanged
