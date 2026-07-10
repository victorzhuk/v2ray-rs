## Why

Changing the auto-resolve strategy while connected silently drops and rebuilds the tunnel ~300ms after the edit — the only settings change that bypasses the "Configuration changed / Apply & Restart" banner every other runtime-relevant edit uses (DNS, routing, manual nodes, subscriptions). No spec mandates the current immediate-disconnect behavior. Source: session gap-scan finding "strategy change force-disconnects instead of using the banner".

## What Changes

- A strategy change while connected sets the restart-required flag and shows the existing banner instead of immediately disconnecting; the reconnect happens on explicit "Apply & Restart".
- `RuntimeConfigSnapshot` starts tracking `auto_resolve_strategy` (and `real_delay.use_for_lowest_latency`, equally strategy-affecting and equally untracked) so the generic divergence check picks strategy changes up.
- Behavior change: previously automatic reconnect becomes consent-gated — matching every other runtime edit.
- Out of scope (agreed in brainstorm): implementing the banner's Discard action. `ui-chrome` promises Discard but it was never wired; that pre-existing drift is a separate change. Snapshot restore of the strategy is included here only so a future Discard can roll it back.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `ui-chrome`: new requirement — strategy changes while connected ride the restart-required banner.
- `connection-auto-resolve`: new requirement — a strategy change takes effect on the next connection; while connected it requires explicit apply, never an automatic disconnect.

## Impact

- `crates/core/src/runtime_snapshot.rs` — add `auto_resolve_strategy` + `use_real_delay_for_lowest_latency` to the snapshot, `diverges_from`, and `restore_settings`.
- `crates/ui/src/app.rs` — delete the strategy special-case (immediate `reconnect_pending` + Disconnect) in `FlushSettings`; the existing `check_restart_required` path takes over.
- Existing banner, `ApplyAndRestart`, and reconnect mechanics unchanged.
