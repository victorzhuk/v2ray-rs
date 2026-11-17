# Spec Delta

## MODIFIED Requirements

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
