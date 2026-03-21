## MODIFIED Requirements

### Requirement: Drag-and-Drop List Reordering
The system SHALL allow drag-and-drop reordering for routing rules, subscriptions, and subscription nodes.

#### Scenario: Drag subscription to reorder sources
- **WHEN** a user drags a subscription row to a new position
- **THEN** the subscription order SHALL update immediately and persist

#### Scenario: Drag node within a subscription
- **WHEN** a user drags a node row within a subscription
- **THEN** the node order SHALL update immediately and persist
