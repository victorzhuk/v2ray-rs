# Spec Delta

## ADDED Requirements

### Requirement: Observatory polling reuses channel and finishes when observations settle
Observatory Real Delay polling SHALL reuse a single loopback gRPC channel across poll iterations, await channel readiness before issuing `GetOutboundStatus`, and return early once every probed outbound tag has reported an observation (or after alive observations settle past the initial probe window) instead of waiting until the full timeout on dead nodes.

#### Scenario: Batch with dead node finishes once observatory reports all tags
- **WHEN** an xray Observatory probe runs with a 5000 ms timeout and xray reports statuses for both a live node (`alive = true, delay = 120`) and a dead node (`alive = false`) within 800 ms
- **THEN** polling SHALL return those results immediately without sleeping the remaining 4200 ms
