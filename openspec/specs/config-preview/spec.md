# config-preview Specification

## Purpose
Gives the user a read-only window onto the exact config file the app generated for the active backend, so what the proxy binary actually consumes can be inspected (and its path shared) without leaving the app. Credential values are masked by default with an explicit reveal, and the view always re-reads from disk rather than a regenerated copy.

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
The preview SHALL mask credential values by default -- JSON values at keys `id`, `uuid`, `password`, `short_id`, and `shortId`, sensitive custom transport headers (`Authorization`, `Proxy-Authorization`, `Cookie`, `Set-Cookie`, `X-Api-Key`, matched case-insensitively), and non-default URL path or query tokens on DoH and H3 DNS server addresses -- while keeping `public_key` visible. Toggling reveal and activating Refresh SHALL preserve the active reveal state, and the on-disk file SHALL never be modified.

#### Scenario: Redacted by default
- **WHEN** the preview opens on a generated config containing proxy credentials, a custom `Authorization` WebSocket header, and a tokenized DoH URL `https://dns.example.com/secret-token?key=abc`
- **THEN** values at `id`, `uuid`, `password`, `short_id`, `shortId`, the `Authorization` header value, and the DoH URL path/query token SHALL be masked while `public_key` remains visible

#### Scenario: Explicit reveal
- **WHEN** the user activates the reveal toggle
- **THEN** the view SHALL show the unmodified file contents

#### Scenario: Refresh preserves reveal state
- **WHEN** the user activates the reveal toggle and then clicks Refresh
- **THEN** the reloaded on-disk file SHALL remain unredacted until the user turns the reveal toggle off

#### Scenario: Redaction failure falls back safely
- **WHEN** the file is not valid JSON and cannot be walked for redaction
- **THEN** the preview SHALL NOT show raw contents by default; it SHALL show an explanatory notice and allow viewing only via the explicit reveal toggle

### Requirement: Empty state when no config exists
The preview SHALL show a distinct empty state explaining why no file exists when the generated config is absent.

#### Scenario: No generated config yet
- **WHEN** the user opens the preview before any config has been generated (no enabled nodes yet, or the runtime directory was cleared)
- **THEN** the dialog SHALL show an empty state naming the expected file location instead of an error
