# config-preview Specification

## Purpose
TBD - created by archiving change add-config-preview. Update Purpose after archive.
## Requirements
### Requirement: View the generated config from the main window
The system SHALL provide a "View Generated Config" action in the main window menu that opens a dialog showing the generated configuration file for the active backend in a monospace, scrollable, read-only view. The dialog SHALL show the literal on-disk file contents (re-read from disk), SHALL offer a Refresh action, and SHALL offer an action that copies the file's path.

#### Scenario: Preview shows the on-disk config
- **WHEN** the user opens View Generated Config and a generated config exists for the active backend
- **THEN** the dialog SHALL display that file's contents as read from disk, not a regenerated in-memory version

#### Scenario: Refresh re-reads the file
- **WHEN** the user activates Refresh in the preview dialog
- **THEN** the view SHALL update to the current on-disk contents

#### Scenario: Copy path
- **WHEN** the user activates the copy action
- **THEN** the file's absolute path SHALL be copied to the clipboard (not the file contents)

### Requirement: Credential redaction with explicit reveal
The preview SHALL mask credential values by default — JSON values at keys `id`, `uuid`, `password`, `short_id`, and `shortId` — and SHALL provide an explicit toggle to reveal the raw contents. `public_key` values SHALL remain visible. Redaction SHALL be display-only: the on-disk file is never modified.

#### Scenario: Redacted by default
- **WHEN** the preview opens
- **THEN** values at keys `id`, `uuid`, `password`, `short_id`, and `shortId` SHALL be masked

#### Scenario: Explicit reveal
- **WHEN** the user activates the reveal toggle
- **THEN** the view SHALL show the unmodified file contents

#### Scenario: Redaction failure falls back safely
- **WHEN** the file is not valid JSON and cannot be walked for redaction
- **THEN** the preview SHALL NOT show raw contents by default; it SHALL show an explanatory notice and allow viewing only via the explicit reveal toggle

### Requirement: Empty state when no config exists
The preview SHALL show a distinct empty state explaining why no file exists when the generated config is absent.

#### Scenario: No generated config yet
- **WHEN** the user opens the preview before any config has been generated (no enabled nodes yet, or the runtime directory was cleared)
- **THEN** the dialog SHALL show an empty state naming the expected file location instead of an error

