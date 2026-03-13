# Proposal: Config Edit Restart Flow

## Why
When the user changes runtime-relevant settings while connected, the edits are persisted but the running backend continues using the previously launched config. The product needs one explicit "restart required" flow instead of mixing persisted edits with silent stale runtime state.

## What Changes
- **Active runtime snapshot**: Capture the exact settings and routing rules used for a connection before `AppMsg::Connect` spawns the backend start task.
- **Pending restart state**: While connected, changes to backend, local ports, DNS, or routing rules remain persisted on disk but mark the runtime config as divergent.
- **Apply or discard**: Show a main-window banner offering "Apply & Restart" or "Discard". Apply reuses the existing disconnect/reconnect flow. Discard restores the persisted settings and routing rules to the last launched snapshot.
- **Disconnected behavior stays immediate**: When the backend is stopped, runtime-relevant edits continue to persist and regenerate config immediately.

## Capabilities

### Modified Capabilities
- `app-persistence`
- `config-generator`
- `process-lifecycle`
- `ui-chrome`
