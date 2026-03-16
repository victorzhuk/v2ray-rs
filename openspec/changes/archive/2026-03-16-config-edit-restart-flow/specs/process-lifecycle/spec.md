## ADDED Requirements

### Requirement: Capture launched runtime snapshot
The system SHALL capture an immutable snapshot of the restart-relevant settings and routing rules that were actually used for the current connection attempt.

#### Scenario: Snapshot captured before launch
- **WHEN** the app prepares the config inputs for `Connect`
- **THEN** it stores the exact settings and routing rules passed to config generation before backend start begins

### Requirement: Apply pending runtime changes by restart
The system SHALL apply pending runtime configuration changes by reusing the normal disconnect/reconnect flow.

#### Scenario: Apply and restart while connected
- **WHEN** the user chooses "Apply & Restart" from the restart-required banner
- **THEN** the system disconnects, reconnects with the already-persisted runtime config, and replaces the active runtime snapshot with the new launched snapshot
