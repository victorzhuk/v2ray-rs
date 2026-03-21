## MODIFIED Requirements

### Requirement: Logs page
The system SHALL keep the current-session log buffer visible even while the backend is stopped.

#### Scenario: Log view while stopped
- **WHEN** no backend process is running
- **THEN** the Logs page SHALL continue showing the most recent in-memory logs together with a "Process not running" indicator

