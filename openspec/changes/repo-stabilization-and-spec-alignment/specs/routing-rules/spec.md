## MODIFIED Requirements

### Requirement: Predefined rule templates
The system SHALL support both built-in presets and user-defined custom presets.

#### Scenario: Save a custom preset
- **WHEN** the user saves the current routing rules as a named preset
- **THEN** the preset SHALL be persisted and available for later application

#### Scenario: Delete a custom preset
- **WHEN** the user deletes a saved custom preset
- **THEN** it SHALL be removed from persisted storage without affecting built-in presets

