# Design: fix-ui-grant-cleanup-and-ux-bugs

## Context

The GTK4/Relm4 UI bridges `v2ray-rs-process` grant/session events and `WorkspaceStore` persistence. Recent anti-DPI UI additions introduced regressions in `grant_confirmed`, `CleanupPending` UX, widget lifetime management, main-thread I/O, and i18n coverage, alongside an existing `WorkspaceStore` bypass on broken-settings reset.

## Decisions

### 1. Accurate `grant_confirmed` for relocating and untargeted grants (`preferences/tun.rs`)
- Compute expected post-grant helper paths before elevation: when `v2ray_rs_process::relocation_required(path)` is true, the expected post-grant path is `RELOCATE_DIR/<bin>` (`relocated_helper_path()` / `relocated_antidpi_path()`); otherwise it is the resolved in-place path.
- Evaluate `let antidpi = antidpi_install_reason(&probe.antidpi);` once:
  - When `antidpi_targeted == true`: require `probe.antidpi.path == targets.antidpi_path` and `matches!(antidpi, AntidpiVerdict::Ready)`.
  - When `antidpi_targeted == false`: allow `AntidpiVerdict::Ready | AntidpiVerdict::Skip | AntidpiVerdict::Fixable(_)` (fail only on `AntidpiVerdict::Terminal(_)`), matching `grant_confirmation_requires_verified_targets` at `tun.rs:1969-1975`.

### 2. Persistent `CleanupPending` recovery UX (`app/mod.rs`, `app/session_bridge.rs`)
- Add a persistent `adw::Banner` (`cleanup_banner`) in the main window with button `"Retry cleanup"` wired to `AppMsg::RetryCleanup`, revealed whenever `self.cleanup_pending == true` and hidden once cleanup clears.
- Disable the Connect button while `self.cleanup_pending == true` (extend `connect_toggle` or apply `!self.cleanup_pending` sensitivity) so clicking Connect is never a silent no-op.
- In `AppMsg::CleanupPending`, set `self.cleanup_pending = true` before calling `apply_state` (or suppress `apply_state`'s generic `"Error: ..."` toast when `cleanup_pending` is true) so the user sees one clear banner/toast instead of two competing toasts.

### 3. Break TUN page `Rc` cycle and remove GTK main-thread FS probes (`preferences/tun.rs`)
- Keep the refresh slot alive in the callback and store only a weak reference to the callback in the slot. Upgrade it to resume a queued refresh. Capture the capability row and Grant button weakly so the click handler cannot keep its own widget alive.
- Move `run_path()` / `wrapper_on_disk` (`tun.rs:754-757`) into the existing `spawn_blocking` task at `tun.rs:788-796`, and move `helper_path()` / `antidpi_path()` (`tun.rs:937, 940`) inside the `spawn_blocking` grant task at `tun.rs:963`.

### 4. `WorkspaceStore::reset_settings()` (`workspace.rs`, `app/mod.rs`)
- Add `WorkspaceStore::reset_settings(&self) -> Result<Arc<AppSettings>, String>` that schedules removal/default-reset of `settings.toml` on the serialized writer thread, updates `snapshot.settings` to `Arc::new(AppSettings::default())`, and broadcasts the updated snapshot.
- Call `self.store.reset_settings()` from `AppMsg::ResetBrokenSettings`, and run the same post-reset sync (`geodata_service`, `subscriptions_page`, preferences state) regardless of whether `settings.toml` existed on disk.

### 5. Strategy editor blank-line rejection and gettext completeness (`preferences/antidpi.rs`, `locale/`)
- In `strategies_from_text` (`antidpi.rs:328`), replace `lines.iter().all(|l| l.is_empty())` with `lines.is_empty() || lines == [""]` so `""` and `"\n"` clear the list while `"\n\n"` returns the blank-line error.
- Add the four missing `msgid` strings (`tun.rs:185, 216, 229, 235`) to `locale/v2ray-rs.pot`, `locale/en_US/LC_MESSAGES/v2ray-rs.po`, and `locale/ru_RU/LC_MESSAGES/v2ray-rs.po` and regenerate `.mo` via `msgfmt`.

## Risks / Trade-offs

- **Persistent cleanup banner**: Takes ~40px vertical space only during an active cleanup failure (never during normal operation) and guarantees recovery stays reachable after any toast times out.
