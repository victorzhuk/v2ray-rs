# Design: DNS UX Rethink

## Context
The canonical DNS model already supports named servers, auto-derived vs custom rules, and replace-only presets. The UX change should simplify the common case without changing those semantics.

## Architecture

### 1. Simplified primary view
- The DNS page shows dedicated `remote` and `domestic` rows as the primary surface.
- If extra servers or nonstandard tags exist, they remain untouched in the full server list inside Advanced.
- If either standard tag is missing, the primary row shows "Not configured" and directs the user to Advanced instead of silently rewriting tags.

### 2. Advanced DNS controls
- The Advanced section contains the full server list, the `use_custom_rules` toggle, the custom-rule editor, FakeIP settings, and sing-box-only detour fields.
- `use_custom_rules` keeps its current config-generation semantics; this change only moves the control.

### 3. Provider presets
- Preset application remains replace-only.
- Applying a preset keeps the confirmation dialog and, on confirmation, replaces `dns.servers` and `dns.strategy` exactly as the canonical preset spec describes.
- No merge mode is introduced in this change.

### 4. Validation
- `DnsConfig::validate()` becomes the single source of truth for live form validation.
- Extend model validation to include FakeIP IPv4/IPv6 CIDR ranges in addition to duplicate tags, invalid rule targets, and invalid client subnet values.
- Fields with validation errors show inline error styling; the erroneous value is NOT persisted (emit is suppressed) until corrected. Dialog Apply buttons (server add/edit, rule add/edit) disable while validation fails.
