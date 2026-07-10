## Why

There is no way to connect to a specific node: the planner only offers strategy-ordered candidate lists, so "switch to that node" today means disabling every other node or reordering and reconnecting. A per-node Connect action was requested during the live-subscription-editing work (session gap-scan / change-3 follow-up).

## What Changes

- A "Connect" action on each node row (subscription nodes and manual nodes) that connects to exactly that node.
- One-shot, zero-fallback semantics (agreed in brainstorm): only the chosen node is tried; failure surfaces immediately. No persistence — the next ordinary Connect uses the configured strategy. Sticky/persisted pinning is explicitly deferred.
- If already connected, the action performs the usual disconnect-then-connect flow to the chosen node.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `connection-auto-resolve`: new requirement — direct connection to a user-chosen node bypassing strategy ordering for that attempt.
- `ui-lists`: new requirement — a per-node Connect action in node row menus.

## Impact

- `crates/ui/src/app.rs` — new `AppMsg::ConnectToNode(ConnectionNodeRef)` handler mirroring `Connect` but passing a single-candidate list; `connection::spawn` needs no changes (already consumes `Vec<ConnectionCandidate>`).
- `crates/ui/src/nodes.rs` — add Connect to the existing per-row popover; new `NodesOutput` variant.
- `crates/ui/src/subscriptions.rs` — subscription node rows have no per-node menu today; add a small per-node affordance (menu or icon button); new `SubscriptionsOutput` variant.
- No changes to `ConnectionPlanner`, `AppSettings`, or persistence.
- Side effect kept: a successful direct connect updates `last_success` like any connect (feeds the Last Successful strategy).
