## ADDED Requirements

### Requirement: GeoData UI Management
The system SHALL provide a Preferences UI for manual geodata refresh that shows last successful refresh time and indexed tag counts for the current backend.

#### Scenario: Manual refresh success
- **WHEN** the user clicks "Update Now" in the GeoData preferences section
- **THEN** the system downloads geodata for the current backend, rebuilds the autocomplete index, and updates the displayed refresh time and tag counts

#### Scenario: Manual refresh failure preserves previous index
- **WHEN** manual refresh or reindex fails
- **THEN** the previous index and displayed metadata remain available and the app reports the failure
