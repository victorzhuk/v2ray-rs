## ADDED Requirements

### Requirement: Subscription node carries Real Delay sample
Each `SubscriptionNode` (and the equivalent manual-node record) SHALL carry an optional `last_real_delay_ms: Option<u64>` field representing the most recent successful Real Delay measurement in milliseconds. The field SHALL default to `None` for newly imported nodes and SHALL round-trip through JSON serialization without affecting other persisted fields.

#### Scenario: New subscription has no Real Delay samples
- **WHEN** the user imports a new subscription
- **THEN** every parsed `SubscriptionNode` SHALL have `last_real_delay_ms = None` until a Real Delay probe runs

#### Scenario: Real Delay sample survives subscription refresh
- **WHEN** a subscription is refreshed (re-fetched) and a previously known node (matched by stable node identity) is still present
- **THEN** the system SHALL preserve that node's `last_real_delay_ms` across the refresh and SHALL NOT reset it to `None`

#### Scenario: Real Delay field round-trips JSON
- **WHEN** a subscription containing nodes with Real Delay samples is serialized to JSON and deserialized again
- **THEN** all `last_real_delay_ms` values SHALL be preserved exactly
