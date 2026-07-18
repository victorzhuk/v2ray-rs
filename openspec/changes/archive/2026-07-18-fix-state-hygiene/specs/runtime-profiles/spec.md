## ADDED Requirements

### Requirement: Instance stamp stays accurate and legacy files are cleaned up
The system SHALL refresh `build_version` in the instance stamp on every start, and legacy-location files (pre-XDG-split `generated/`, `geodata/`, PID and snapshot files under `data_dir`) SHALL be migrated to their current locations with the destination directory created as needed. When the destination already holds current data, the legacy copies SHALL be deleted rather than retained; generated configs contain node credentials and MUST NOT linger in abandoned locations.

#### Scenario: Stamp reflects the running build
- **WHEN** the app starts with an instance stamp written by an older build
- **THEN** after startup the stamp's `build_version` SHALL equal the running build's version

#### Scenario: Relocation into a missing destination directory
- **WHEN** legacy `data_dir/generated/` files exist and `runtime_dir/generated/` does not yet exist
- **THEN** the relocation SHALL create the destination directory and move the files, leaving the legacy directory removed

#### Scenario: Populated destination still clears legacy copies
- **WHEN** legacy `data_dir/generated/` files exist and `runtime_dir/generated/` already contains current configs
- **THEN** the legacy files SHALL be deleted
