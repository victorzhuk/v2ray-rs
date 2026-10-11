## ADDED Requirements

### Requirement: Node rows show egress location
Subscription node rows SHALL display the node's last known egress sample (location code and address) when one exists, and SHALL display nothing when it does not. Nodes in different subscriptions carrying identical display names SHALL be distinguishable by their shown egress.

#### Scenario: Egress shown after a probe session
- **WHEN** a probe session records egress `AT` and `2001:db8::1` for a node
- **THEN** the node row SHALL show the egress location and address without requiring a list rebuild or disconnecting the backend

#### Scenario: Identical names with different exits are distinguishable
- **WHEN** two subscription nodes both labelled `Germany - Nuremberg` carry egress samples `DE` and `AT` respectively
- **THEN** the two rows SHALL show different egress values matching each node's own sample

#### Scenario: No sample shows nothing
- **WHEN** a node has never been probed or its last egress attempt failed with no prior sample
- **THEN** the node row SHALL NOT show an egress placeholder or an empty egress label
