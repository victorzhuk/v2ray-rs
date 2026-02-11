## ADDED Requirements

### Requirement: Manual subscription update
The system SHALL allow the user to manually trigger a re-fetch and re-parse of any subscription.

#### Scenario: Manual update success
- **WHEN** the user clicks "Update" on a subscription
- **THEN** the system SHALL re-fetch the URL, re-parse nodes, and replace the old node list while preserving per-node enable/disable preferences for nodes that still exist

### Requirement: Automatic subscription update
The system SHALL support automatic periodic updates of subscriptions based on a configurable interval.

#### Scenario: Auto-update triggers
- **WHEN** the configured auto-update interval elapses
- **THEN** the system SHALL fetch and update all subscriptions with auto-update enabled

#### Scenario: Auto-update failure
- **WHEN** an auto-update fails due to network error
- **THEN** the system SHALL retry up to 3 times with exponential backoff and log the failure

### Requirement: Update node reconciliation
The system SHALL reconcile updated subscription nodes with existing data, matching nodes by their address+port+protocol to preserve user preferences.

#### Scenario: Node added in update
- **WHEN** a subscription update contains a new node not in the previous version
- **THEN** the new node SHALL be added with enabled status by default

#### Scenario: Node removed in update
- **WHEN** a subscription update no longer contains a previously existing node
- **THEN** that node SHALL be removed from the subscription
