## ADDED Requirements

### Requirement: Release acceptance exercises both observatory backends
Release acceptance SHALL exercise the production Real Delay path against installed xray and v2ray binaries under a process deadline.

#### Scenario: Mixed local probe batch
- **WHEN** each backend measures a reachable fixture and an unavailable fixture
- **THEN** only the reachable node SHALL receive a delay and the probe process SHALL exit before the deadline

#### Scenario: Missing prerequisite
- **WHEN** a required backend or fixture cannot start
- **THEN** the release acceptance job SHALL fail and identify the missing prerequisite
