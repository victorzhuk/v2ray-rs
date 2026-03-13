## ADDED Requirements

### Requirement: Restore persisted runtime configuration
The system SHALL be able to restore persisted settings and routing rules to the last launched runtime snapshot when the user discards connected-state changes.

#### Scenario: Discard connected edits
- **WHEN** the user selects "Discard" from the restart-required banner
- **THEN** the system overwrites the current persisted settings and routing rules with the active runtime snapshot and clears the pending divergence state
