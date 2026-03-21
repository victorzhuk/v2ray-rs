## MODIFIED Requirements

### Requirement: Auto-update geodata
The system SHALL use the persisted geodata refresh settings to perform background refreshes while the app is running.

#### Scenario: Startup refresh when data is missing
- **WHEN** the app starts and the selected backend has no local geodata
- **THEN** the app SHALL attempt a background download and reindex pass without blocking startup

#### Scenario: Scheduled refresh while running
- **WHEN** `auto_update_geodata` is enabled and the configured interval elapses
- **THEN** the app SHALL check for updates and rebuild the index if new files arrive or the index is missing

