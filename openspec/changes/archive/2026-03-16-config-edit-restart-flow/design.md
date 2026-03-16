# Design: Config Edit Restart Flow

## Context
Settings already auto-persist, but the running backend is launched from a point-in-time config assembled in `AppMsg::Connect`. The change needs to track that launched runtime input and expose when the persisted config has diverged.

## Architecture

### 1. Runtime snapshot ownership
- Add a `RuntimeConfigSnapshot` owned by `App` containing the exact restart-relevant settings plus the `RoutingRuleSet` used for the last successful launch.
- Capture the snapshot immediately before spawning the connect task, using the same settings and routing rules passed to config generation.
- Exclude non-runtime fields such as `last_success`, language, tray, and notifications.

### 2. Preferences-to-app change propagation
- Extend the Preferences integration so routing-rule mutations notify `App`, instead of existing only inside dialog-local state.
- `App` compares the persisted current config against the active snapshot and sets `restart_required` only while the backend is `Starting` or `Running`.
- If the backend is stopped, no divergence banner is shown.

### 3. Banner actions
- `Apply & Restart` clears the banner, then reuses the existing disconnect/reconnect path so the next `Connect` uses the already-persisted config and refreshes the snapshot.
- `Discard` overwrites persisted settings and routing rules with the active snapshot and closes any open Preferences dialog before it can re-save stale widget state.
- Disconnecting without applying also clears the banner because there is no longer an active runtime snapshot.

### 4. Config generation policy
- Disconnected edits keep the current eager regeneration behavior.
- Connected edits do not replace the running config file in-place. The regenerated file is produced when the user applies restart or reconnects later.

## Data Flow
`Preferences edit` -> persisted settings/rules update -> `App` compares against `RuntimeConfigSnapshot` -> banner -> `Apply & Restart` or `Discard`

## Alternatives Considered
- Auto-restarting on every runtime-relevant edit: rejected because it causes unexpected disconnects.
- Comparing raw `AppSettings`: rejected because non-runtime fields would cause false divergence.
