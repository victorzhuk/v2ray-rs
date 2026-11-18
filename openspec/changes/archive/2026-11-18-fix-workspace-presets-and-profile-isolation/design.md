# Design

## Context

See `proposal.md` for motivation:
- `WorkspaceStore` (`crates/ui/src/workspace.rs`) serializes `update_settings`, `update_subscriptions`, `update_manual_nodes`, and `update_routing_rules` under `self.lock`, whereas `persist_latency_snapshot` (`crates/ui/src/subscriptions/probes.rs:69-87`) calls `load_latency_snapshot()` and `save_latency_snapshot()` as two separate lock acquisitions across concurrent `spawn_blocking` tasks.
- `crates/core/src/persistence/presets.rs` maps `"My Preset"`, `"my-preset"`, and `"My_Preset!"` to `my-preset.json`; `save_preset` blindly overwrites and `delete_preset` unlinks `my-preset.json` before checking `preset.name`.
- `crates/ui/src/app/bootstrap.rs:471` skips `try_activate_running_instance` when `overrides.has_directory_overrides()` is true, yet `adw::Application::builder().application_id(&app_id).build()` at line 586 still uses default `GApplication` uniqueness, causing GIO to forward `activate` to the primary D-Bus owner and exit `0`.
- `crates/tray/src/tray.rs:54` returns a constant `"v2ray-rs"` from `Tray::id()`.

## Goals / Non-Goals

**Goals:**
- Make every latency snapshot read-modify-write atomic inside `WorkspaceStore`.
- Guarantee 1-to-1 preset persistence by exact `preset.name` while keeping existing single-slug files readable and updatable in place.
- Ensure `--*-dir` overrides work even when the base profile is running on the same desktop session.
- Give each profile/instance a distinct SNI `id` in `v2ray-rs-tray`.

**Non-Goals:**
- Changing the JSON schema of `latency_snapshot.json` or custom preset files.

## Decisions

1. **Add `WorkspaceStore::update_latency_snapshot`.**
   Implement `pub fn update_latency_snapshot<R>(&self, mutate: impl FnOnce(&mut LatencySnapshot) -> R) -> Result<R, PersistenceError>` acquiring `self.lock()`, cloning the cached `Arc<LatencySnapshot>` (or mutating a working copy), running `mutate`, calling `self.persist_latency(updated)` only when changed (or unconditionally for mutations), and returning `Ok(result)`. Replace `persist_latency_snapshot` in `probes.rs` and the prune save in `app/mod.rs:1363-1373` (plus coalesce the `for` loop in `AutoUpdateDone` at `subscriptions/mod.rs:1299-1308` into one `update_latency_snapshot` pass).

2. **Collision-safe preset path resolution in `presets.rs`.**
   - Candidate primary path remains `{slug}.json`.
   - On `save_preset`: if `{slug}.json` does not exist, or exists and deserializes to a `Preset` with `existing.name == preset.name`, write `{slug}.json`. Otherwise write `{slug}-{fnv1a_64(preset.name):016x}.json` (also updating in place if an existing file already holds `preset.name`).
   - On `delete_preset(paths, name)`: check `{slug}.json` first **only** if its parsed `preset.name == name`; otherwise scan `{dir}/*.json` (including `{slug}-{hash}.json` and legacy `preset-{hash}.json`) for `preset.name == name` and unlink the exact match.

3. **Set `gio::ApplicationFlags::NON_UNIQUE` when `overrides.has_directory_overrides()`.**
   Configure `adw::Application::builder().application_id(&app_id).flags(...)` with `gio::ApplicationFlags::NON_UNIQUE` iff `overrides.has_directory_overrides()` is `true`. `InstanceLock::acquire(&paths)` at `bootstrap.rs:475` remains the single-instance authority for that overridden `runtime_dir`.

4. **Pass tray identifier from `bootstrap.rs` into `TrayService::spawn`.**
   Extend `TrayService::spawn` (or `TrayConfig`) with `tray_id: String` derived from the profile qualifier (e.g., `"v2ray-rs"` for `Production`, `profile.qualifier()` such as `"v2ray-rs-dev"` / `"v2ray-rs-qa"` for other profiles, and appending a short deterministic suffix when directory overrides are active).

## Risks / Trade-offs

- **[Risk] Corrupt `{slug}.json` blocking overwrite of the same preset** -> A corrupt or foreign file is never overwritten: `save_preset` consolidates an existing file only when its stored name matches and otherwise reserves a name-derived hashed path after verifying ownership, so a corrupt file is neither silently clobbered nor a blocker.
- **[Risk] Live GApplication/D-Bus registration is not exercised by automated tests** -> The flag selection is unit-tested and `InstanceLock` still governs the overridden directory set, but Xvfb aborts in the development sandbox, so an overridden instance starting alongside a primary owner was not observed on this host. The CI job that already runs under `xvfb-run -a dbus-run-session` is the place to add that test if its flake budget allows.
- **[Risk] Two processes sharing one data directory can race preset reservation** -> Preset persistence stays last-writer-wins like every other workspace file; the instance lock prevents this for normal launches, and the supported concurrency boundary is one instance per profile plus directory-override instances with distinct directory sets.
