## ADDED Requirements

### Requirement: Per-node Connect action
Node rows in the subscription and manual node lists SHALL offer a Connect action for enabled nodes that triggers a direct connection to that node. The action SHALL be unavailable (hidden or insensitive) for disabled nodes.

#### Scenario: Connect from a manual node row
- **WHEN** the user opens an enabled manual node's row menu and chooses Connect
- **THEN** a direct connection to that node starts

#### Scenario: Connect from a subscription node row
- **WHEN** the user activates the Connect affordance on an enabled subscription node row
- **THEN** a direct connection to that node starts

#### Scenario: Disabled node offers no Connect
- **WHEN** a node is disabled
- **THEN** its row SHALL NOT offer an actionable Connect
