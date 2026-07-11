## Context

`connection::spawn` already consumes an ordered `Vec<ConnectionCandidate>` and treats it as a fallback chain — a one-element list gives zero-fallback semantics for free. `ConnectionNodeRef` (subscription_id + node_id, or manual node_id) already identifies nodes stably. Manual node rows have a per-row popover (Edit/Delete); subscription node rows have no menu — only switch, drag handle, badges.

## Goals / Non-Goals

- Goal: smallest correct path — a single-candidate connect reusing the whole existing pipeline.
- Non-goal: sticky/persisted pin (own change later: AppSettings field, wire struct, planner precedence, unpin UI).
- Non-goal: fallback-behind-the-pin ordering (rejected in brainstorm — label honesty).

## Decisions

- New `AppMsg::ConnectToNode(ConnectionNodeRef)`; handler mirrors `Connect` (load, snapshot, regenerate) but resolves the single node to one `ConnectionCandidate` instead of `planner.plan()`. Alternative — teaching `ConnectionPlanner` a pin parameter — rejected: planner stays strategy-only; pinning is a UI-initiated one-shot.
- Already-connected case reuses the `reconnect_pending` mechanism (set pending intent, dispatch Disconnect, connect on Stopped) — same flow as Apply & Restart, but the pending connect must target the chosen node, e.g. a `pending_connect: Option<ConnectionNodeRef>` consumed where `reconnect_pending` is consumed today.
- Disabled nodes: action hidden/insensitive rather than auto-enabling — enabling is a persisted state change the user didn't ask for.
- Subscription rows: prefer a compact icon button suffix over a full popover to respect the ui-lists "max two visible suffix widgets" requirement — final widget choice at implementation, requirement stays widget-agnostic.

## Risks / Trade-offs

- [Zero-fallback surprises users expecting failover] → the label says "Connect to this node"; error surfaces immediately with the node name.
- [Direct connect updates `last_success`] → intentional; documented in the spec scenario.

## Migration Plan

Single PR; UI-only + one message enum addition. Rollback = revert.

## Open Questions

None.
