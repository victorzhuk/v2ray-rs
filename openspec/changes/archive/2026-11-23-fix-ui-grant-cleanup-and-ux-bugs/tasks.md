# Tasks: fix-ui-grant-cleanup-and-ux-bugs

## 1. Grant confirmation fixes (`crates/ui/src/preferences/tun.rs`)

- [x] 1.1 Fix `grant_confirmed` (`crates/ui/src/preferences/tun.rs:326-357`): evaluate `antidpi_install_reason(&probe.antidpi)` once, require `AntidpiVerdict::Ready` + matching path when `antidpi_targeted == true`, and reject only `AntidpiVerdict::Terminal(_)` when `antidpi_targeted == false`.
- [x] 1.2 Compute expected post-grant destination paths in `GrantTargets` (`tun.rs:937-957`) using relocated paths under `RELOCATE_DIR` when relocation is required so first-time AppImage/`nosuid` grants confirm cleanly.
- [x] 1.3 Add unit tests for relocating `GrantTargets` confirmation alongside the existing `grant_confirmation_requires_verified_targets` test (`tun.rs:1954-1975`).

## 2. `CleanupPending` persistent UX (`crates/ui/src/app/mod.rs`, `session_bridge.rs`)

- [x] 2.1 Add a persistent `cleanup_banner: adw::Banner` with `"Retry cleanup"` wired to `AppMsg::RetryCleanup` in `crates/ui/src/app/mod.rs`, shown while `self.cleanup_pending` is true and hidden when cleanup clears.
- [x] 2.2 Keep the Connect button insensitive while `self.cleanup_pending` is true and suppress the duplicate `"Error: ..."` toast from `apply_state` (`app/mod.rs:240-252, 1899-1926`).

## 3. Widget leak, GTK main-thread blocking, `WorkspaceStore` reset, editor, and i18n

- [x] 3.1 Break TUN capability refresh cycles with a weak callback back-reference and weak GTK widget captures, preserving queued refreshes.
- [x] 3.2 Move `run_path()` (`tun.rs:754-757`) and `helper_path()`/`antidpi_path()` (`tun.rs:937, 940`) into `spawn_blocking` tasks off the GTK main thread.
- [x] 3.3 Add `WorkspaceStore::reset_settings()` in `crates/ui/src/workspace.rs`, route `AppMsg::ResetBrokenSettings` (`app/mod.rs:2083-2122`) through it, and run identical post-reset syncing for both `Ok` and `NotFound`.
- [x] 3.4 Fix `strategies_from_text` (`crates/ui/src/preferences/antidpi.rs:328`) so `"\n\n"` returns an error instead of clearing saved strategies, and add a unit test.
- [x] 3.5 Add the 4 missing anti-DPI capability `msgid` strings (`tun.rs:185, 216, 229, 235`) to `locale/v2ray-rs.pot`, `locale/en_US/LC_MESSAGES/v2ray-rs.po`, and `locale/ru_RU/LC_MESSAGES/v2ray-rs.po`, and regenerate `.mo` catalogs with `msgfmt`.

## 4. Verification

- [x] 4.1 Run `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4` (including `grant_confirmation_requires_verified_targets`, relocating grant test, `strategies_from_text`, and `WorkspaceStore::reset_settings`).
- [x] 4.2 Run `make fmt`, `make clippy`, and `scripts/stage-dist.sh` catalog verification (`msgunfmt`) confirming all 4 new Russian translations are staged.
