# Spec Delta

## MODIFIED Requirements

### Requirement: Edit and delete routing rules
The system SHALL allow users to modify existing rules' match conditions and actions while preserving the rule's `enabled` state and optional `group` metadata, and to delete rules.

#### Scenario: Change rule action
- **WHEN** the user changes a rule's action from "proxy" to "direct"
- **THEN** the system SHALL update the rule and trigger config regeneration

#### Scenario: Editing a disabled or grouped rule preserves metadata
- **WHEN** an existing routing rule has `enabled = false` or `group = Some("work")` and the user edits its match condition or action in the rule dialog
- **THEN** the updated rule SHALL retain `enabled = false` and `group = Some("work")`

#### Scenario: Delete rule
- **WHEN** the user deletes a routing rule
- **THEN** the rule SHALL be removed and config SHALL be regenerated
