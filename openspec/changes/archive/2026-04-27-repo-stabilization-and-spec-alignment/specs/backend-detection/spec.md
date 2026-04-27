## MODIFIED Requirements

### Requirement: Auto-detect installed backends
The system SHALL keep detected backend binaries visible even when version probing fails, marking them unavailable instead of silently omitting the failure.

#### Scenario: Version probe failure remains visible
- **WHEN** a backend binary exists but `version` probing fails
- **THEN** the backend remains listed in onboarding/preferences, is disabled for selection, and displays the probe error

#### Scenario: Single usable backend installed
- **WHEN** exactly one detected backend is available for use
- **THEN** onboarding SHALL auto-select that backend

### Requirement: Custom backend path
The system SHALL validate custom backend paths strictly before accepting them.

#### Scenario: Version probe fails for custom path
- **WHEN** the user enters an executable custom path whose `version` command fails
- **THEN** the system SHALL reject the path and show the validation error instead of saving it

