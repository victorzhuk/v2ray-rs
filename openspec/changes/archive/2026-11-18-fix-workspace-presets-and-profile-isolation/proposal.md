# Proposal

## Why

Concurrent latency snapshot tasks perform unlocked load-modify-save cycles that clobber sibling probe, Real Delay, egress, and verdict updates; custom routing presets whose names normalize to the same ASCII slug overwrite or delete one another; and CLI directory overrides (`--config-dir`, `--data-dir`, `--runtime-dir`) still register a unique `GApplication` on D-Bus (exiting into the primary instance) while sharing a hardcoded `"v2ray-rs"` tray item ID across profiles.

## What Changes

- Add an atomic `WorkspaceStore::update_latency_snapshot` operation holding the store mutex across read, mutation, and persistence, and migrate all probe, invalidation, and connect-prune callers to it.
- Preserve distinct custom routing presets when `slugify` collides on different `preset.name` values by disambiguating filenames with the deterministic FNV-1a hash suffix and verifying `preset.name` before overwrite or deletion.
- Set `gio::ApplicationFlags::NON_UNIQUE` on `adw::Application` when per-directory path overrides are active so overridden instances launch independently alongside a running default profile instance while enforcing `InstanceLock`.
- Scope the StatusNotifierItem `AppTray::id()` to the active profile / application ID instead of hardcoding `"v2ray-rs"`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `app-persistence`: Serialize read-modify-write latency snapshot updates atomically and prevent custom preset slug collisions from overwriting or deleting another preset.
- `runtime-profiles`: Allow directory-overridden CLI instances to run concurrently alongside a primary profile instance without session-bus uniqueness hijacking startup.
- `system-tray`: Scope the tray StatusNotifierItem identifier to the active profile.

## Impact

`crates/core/src/persistence/presets.rs`, `crates/ui/src/{workspace.rs,subscriptions/probes.rs,subscriptions/mod.rs,app/mod.rs,app/bootstrap.rs}`, and `crates/tray/src/tray.rs`. Backward-compatible with existing `latency_snapshot.json` and single-slug preset files.
