## MODIFIED Requirements

### Requirement: Global auto-resolve strategy setting
The current supported strategies SHALL be list order, lowest latency, random, and last successful.

#### Scenario: Legacy geo-aware setting
- **WHEN** persisted settings still contain `geo-aware`
- **THEN** the app SHALL migrate that value to `last-successful` on load

