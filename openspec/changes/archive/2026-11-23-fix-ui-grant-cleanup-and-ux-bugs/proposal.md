# Proposal: fix-ui-grant-cleanup-and-ux-bugs

## Why

Review of `crates/ui` and `locale/` surfaced two grant-verification bugs, a broken recovery UX for `CleanupPending`, a widget reference-cycle leak, GTK main-thread blocking, a `WorkspaceStore` bypass, missing translations, and a strategy-editor validation bug:

- `grant_confirmed` (`crates/ui/src/preferences/tun.rs:349-357`) contradicts `antidpi_targeted` and fails its own unit test (`tun.rs:1954-1975`): when `antidpi_targeted == false` and the anti-DPI helper returns `AntidpiVerdict::Fixable(_)`, the trailing `!matches!(...)` still returns `false`.
- `grant_confirmed` (`tun.rs:336-338, 346-348, 937-957`) always fails on first-time relocating grants (AppImage / `nosuid` mounts): `targets` captures the pre-grant bundled paths, while post-grant `probe_helper()` resolves the newly relocated `/usr/local/lib/v2ray-rs/*` paths, triggering a false `"Grant did not fully verify"` error after a successful grant.
- `AppMsg::CleanupPending` (`crates/ui/src/app/mod.rs:240-252, 1528-1537, 1899-1926`) leaves the Connect button enabled as a silent no-op, fires a duplicate `"Error: ..."` toast from `apply_state` before the cleanup toast, and exposes `"Retry cleanup"` only on an auto-dismissing toast -- leaving the app dead-ended until restart once the toast disappears.
- `build_tun_page` (`tun.rs:861-925`) creates a strong `Rc` cycle between `spawn_refresh` and the `spawn` closure capturing `cap_row`, `grant_button`, and `state`, leaking all captured widgets on every Preferences dialog open.
- `build_tun_page` (`tun.rs:754-757`) and `grant_button.connect_clicked` (`tun.rs:937, 940`) call `run_path()`, `helper_path()`, and `antidpi_path()` synchronously on the GTK main thread, performing `/proc/self/mounts` and `$PATH` filesystem I/O.
- `AppMsg::ResetBrokenSettings` (`app/mod.rs:2083-2122`) deletes `settings.toml` directly via `std::fs::remove_file` without updating `WorkspaceStore`'s cached snapshot, and its `NotFound` arm skips syncing `geodata_service` and `subscriptions_page`.
- Four new `tr(...)` anti-DPI capability strings (`tun.rs:185, 216, 229, 235`) are missing from `locale/v2ray-rs.pot`, `en_US`, and `ru_RU` catalogs.
- `strategies_from_text` (`crates/ui/src/preferences/antidpi.rs:315-341`) accepts multi-newline blank inputs (`"\n\n"`) as clearing the saved strategy list instead of rejecting blank lines.

## What Changes

- Fix `grant_confirmed` to honor `antidpi_targeted` (checking `antidpi_install_reason` once) and compare post-grant probe paths against expected post-grant destination paths (relocated under `/usr/local/lib/v2ray-rs` when relocation is required, else in-place).
- Add a persistent cleanup-pending `adw::Banner` with a `"Retry cleanup"` button in `App`, disable Connect while `cleanup_pending` is true, and suppress the duplicate `"Error: ..."` toast from `apply_state`.
- Break the `Rc` reference cycle in `preferences/tun.rs` with `Weak` (`Rc::downgrade`) and move `run_path()`, `helper_path()`, and `antidpi_path()` off the GTK main thread into `spawn_blocking`.
- Add `WorkspaceStore::reset_settings()` to remove/reset `settings.toml` on the serialized writer thread and update the cached `Arc<AppSettings>` snapshot; unify post-reset UI syncing for both `Ok` and `NotFound`.
- Fix `strategies_from_text` so only `""` or a single `"\n"` clears the strategy list while `"\n\n"` is rejected as a blank line.
- Add the four missing anti-DPI capability `msgid` entries to `locale/v2ray-rs.pot`, `locale/en_US/LC_MESSAGES/v2ray-rs.po`, and `locale/ru_RU/LC_MESSAGES/v2ray-rs.po` and recompile `.mo` catalogs.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `tun-preferences-ui`: Grant confirmation handles relocating destinations and untargeted anti-DPI verdicts accurately; capability refreshes do not leak widgets or block the GTK main thread; all capability verdicts are localized.
- `main-window`: `CleanupPending` presents a persistent retry control without duplicate toasts or a silent Connect button; broken-settings reset flows through `WorkspaceStore`.

## Impact

- **Modified crate**: `crates/ui` (`preferences/tun.rs`, `preferences/antidpi.rs`, `app/mod.rs`, `app/session_bridge.rs`, `workspace.rs`).
- **Modified locales**: `locale/v2ray-rs.pot`, `locale/en_US/LC_MESSAGES/v2ray-rs.{po,mo}`, `locale/ru_RU/LC_MESSAGES/v2ray-rs.{po,mo}`.
- **Dependencies**: None new.
