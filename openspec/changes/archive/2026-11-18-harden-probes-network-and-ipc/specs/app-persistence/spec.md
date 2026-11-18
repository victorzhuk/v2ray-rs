# Spec Delta

## ADDED Requirements

### Requirement: Pasted subscription blobs write atomically with owner-only permissions
When saving pasted subscription or configuration-bundle content under `subscription_blobs/`, the system SHALL create and write the file atomically with owner-only `0600` permissions from file creation rather than writing with default umask permissions before changing mode.

#### Scenario: Pasted JSON bundle has mode 0600 throughout creation
- **WHEN** a user imports a subscription from pasted JSON content
- **THEN** the persisted blob under `subscription_blobs/` SHALL be written atomically with mode `0600` and SHALL NOT pass through a group-readable or world-readable state
