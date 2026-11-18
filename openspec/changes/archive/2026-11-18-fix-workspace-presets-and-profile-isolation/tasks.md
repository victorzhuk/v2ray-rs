# Tasks

## 1. Atomic Latency Snapshot Updates and Collision-Safe Presets

- [x] 1.1 Add `WorkspaceStore::update_latency_snapshot` in `crates/ui/src/workspace.rs`, migrate `persist_latency_snapshot` (`crates/ui/src/subscriptions/probes.rs`), `AutoUpdateDone` sample invalidation (`crates/ui/src/subscriptions/mod.rs`), and connect-time pruning (`crates/ui/src/app/mod.rs`) to it, and verify with multi-threaded concurrency tests in `workspace.rs` that parallel updates never drop sibling entries.
- [x] 1.2 Update `save_preset` and `delete_preset` in `crates/core/src/persistence/presets.rs` to verify stored `preset.name` and disambiguate colliding slugs via deterministic FNV-1a hash suffixes, and verify with unit tests covering `"My Preset"` vs `"my-preset"` save, in-place update, reload, and selective delete.

## 2. Directory-Override GIO Isolation and Profile-Scoped Tray ID

- [x] 2.1 Apply `gio::ApplicationFlags::NON_UNIQUE` in `crates/ui/src/app/bootstrap.rs` when `overrides.has_directory_overrides()` is true, and verify with a headless/Xvfb or `GApplication` registration test that an overridden instance initializes alongside a primary D-Bus owner while duplicate launches on the same overridden `runtime_dir` still exit 75.
- [x] 2.2 Pass a profile-scoped tray ID into `TrayService::spawn` (`crates/tray/src/tray.rs` and `crates/ui/src/app/bootstrap.rs`), and verify with unit tests in `crates/tray` that `Production`, `Development`, `Custom`, and directory-overridden instances produce distinct `Tray::id()` strings.
