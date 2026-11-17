# Spec Delta

## ADDED Requirements

### Requirement: Disabled subscriptions cannot supply detour nodes
When resolving a node's `via_node` detour reference during connection planning or config generation, the system SHALL consider only enabled manual nodes and enabled nodes belonging to enabled subscriptions; a target inside a disabled subscription SHALL fail detour resolution.

#### Scenario: Detour target in a disabled subscription is skipped
- **WHEN** an enabled node sets `via_node` to a node inside a subscription with `enabled = false`
- **THEN** detour resolution SHALL treat that target as unavailable and SHALL NOT emit an outbound using the disabled subscription's node
