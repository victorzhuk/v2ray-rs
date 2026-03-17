### Requirement: Drag-and-Drop List Reordering
The system SHALL allow users to reorder routing rules via drag-and-drop interactions inside the existing Preferences page.

#### Scenario: Drag rule to change precedence
- **WHEN** a user clicks and drags a routing rule row up or down in the list
- **THEN** the rule is visually reordered and the underlying configuration updates its position

#### Scenario: Drag-and-drop persists and regenerates
- **WHEN** a drag-and-drop reorder completes
- **THEN** the system saves the updated routing rules and triggers the same config-regeneration path used by the existing move buttons
