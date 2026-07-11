## Context

`FlushSettings` special-cases `strategy_changed`: when connected it sets `reconnect_pending = true` and dispatches Disconnect immediately — the same mechanism `ApplyAndRestart` uses, minus the consent click. The generic path (`check_restart_required` → banner) can't see strategy changes because `RuntimeConfigSnapshot` doesn't track `auto_resolve_strategy` (nor `real_delay.use_for_lowest_latency`). The banner's Discard action is spec'd in `ui-chrome` but was never implemented (`restore_*` helpers are dead code outside tests) — pre-existing drift, explicitly out of scope here.

## Goals / Non-Goals

- Goal: one consistent consent-gated pattern for every runtime-relevant edit while connected.
- Non-goal: implementing Discard (separate change; this change only makes the snapshot able to restore the strategy).
- Non-goal: changing strategy semantics or candidate ordering.

## Decisions

- Track both `auto_resolve_strategy` and `real_delay.use_for_lowest_latency` in the snapshot — the latter changes Lowest Latency ordering identically and is equally untracked today; leaving it out reintroduces the same class of silent divergence. Alternative — strategy only — rejected for that reason.
- Delete the `strategy_changed` special-case entirely; `self.restart_required = self.check_restart_required()` already runs in the same handler and now naturally detects the divergence. No new messages or flags.
- Extend `restore_settings` to roll the strategy back so a future Discard change is purely UI work.

## Risks / Trade-offs

- [Users accustomed to instant strategy apply] → consent-gated is the established pattern for every other edit; the banner is immediate and one click away. Changelog entry flags the behavior change.
- [Snapshot shape change] → in-memory only (no persistence of the snapshot); no migration.

## Migration Plan

Single PR; no data migration. Rollback = revert.

## Open Questions

None.
