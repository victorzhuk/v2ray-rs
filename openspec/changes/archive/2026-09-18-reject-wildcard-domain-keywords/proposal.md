## Why

A domain keyword is a substring matcher, so `*.ru` is not a wildcard suffix and never matches as users expect. Existing persisted rules must remain recoverable rather than being silently rewritten.

## What Changes

- Reject `*` in newly created or edited domain keyword rules.
- Preserve stored invalid keyword rules and mark them invalid in routing and DNS preference rows.
- Render validation error text as the invalid row subtitle.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `routing-rules`: reject wildcard keywords and surface stored invalid rules.
- `dns-preferences-ui`: validate DNS keyword values and surface stored invalid rules.

## Impact

- Core validation, routing persistence, and routing/DNS preference UI.
- No generated-config behavior changes.
