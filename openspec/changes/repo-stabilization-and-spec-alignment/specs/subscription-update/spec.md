## MODIFIED Requirements

### Requirement: Update node reconciliation
The system SHALL reconcile updated subscription nodes with existing data, matching nodes by address, port, and protocol to preserve user preferences.

#### Scenario: Matching node keeps enabled state
- **WHEN** an updated subscription still contains a previously known node
- **THEN** the system SHALL preserve that node's enabled state

### Requirement: Manual subscription update
The system SHALL surface partial parse failures without discarding valid nodes.

#### Scenario: Manual update with mixed valid and invalid URIs
- **WHEN** an update source returns valid proxy URIs together with invalid entries
- **THEN** the system SHALL keep the valid nodes, report the skipped entries, and persist the reconciled result

