# Tasks

## 1. Profile resolution

- [x] 1.1 Add `AppProfile` enum (`Production`, `Development`, `Test`, `Custom(String)`) in `crates/core/src/profile.rs` with `qualifier()`, `app_id_suffix()`, and `parse(&str)` helpers; reject custom names not matching `[a-z0-9][a-z0-9_-]{0,30}`
- [x] 1.2 Implement `AppProfile::resolve(cli: Option<&str>, env: &dyn Env)` honoring CLI > `V2RAY_RS_PROFILE` > legacy `V2RAY_RS_DEV` > compile-time default
- [x] 1.3 Emit a `WARN` log when `V2RAY_RS_DEV` is observed and document its removal target in `CHANGELOG.md`
- [x] 1.4 Unit-test resolution precedence, custom-name validation, and qualifier/app-id derivation

## 2. AppPaths layout extension

- [x] 2.1 Extend `AppPaths` with `cache_dir`, `runtime_dir`, `state_dir` fields and accessors backed by `directories::ProjectDirs` + `BaseDirs`
- [x] 2.2 Implement `runtime_dir` fallback to `data_dir/runtime` when `XDG_RUNTIME_DIR` is unset
- [x] 2.3 Implement `state_dir` fallback to `data_dir/state` when `BaseDirs::state_dir()` is `None`
- [x] 2.4 Add `AppPaths::for_profile(profile)` and `AppPaths::for_profile_in(profile, root)` (no `cfg` gating; replaces the test-only `from_paths`)
- [x] 2.5 Update `ensure_dirs()` to create config/data/cache/runtime/state with `0o700` permissions
- [x] 2.6 Add accessors for new file locations: `pid_file_path()`, `generated_dir()`, `geodata_dir()` (now under cache), `geodata_index_dir()`, `latency_snapshot_path()` (now under state), `instance_stamp_path()`, `instance_lock_path()`
- [x] 2.7 Update unit tests in `persistence/mod.rs` and add coverage for the new accessors and fallbacks

## 3. Per-directory overrides (CLI + env)

- [x] 3.1 Add a `clap`-based CLI parser in the UI binary entry point with `--profile`, `--config-dir`, `--data-dir`, `--cache-dir`, `--runtime-dir`, `--state-dir`, `--reset-instance`, `--install-icons`
- [x] 3.2 Add matching env vars `V2RAY_RS_PROFILE`, `V2RAY_RS_CONFIG_DIR`, `V2RAY_RS_DATA_DIR`, `V2RAY_RS_CACHE_DIR`, `V2RAY_RS_RUNTIME_DIR`, `V2RAY_RS_STATE_DIR`, `V2RAY_RS_INSTALL_ICONS`
- [x] 3.3 Implement `AppPaths::with_overrides(profile, overrides)` applying CLI > env > profile defaults per-directory
- [x] 3.4 Validate override paths: must be absolute or expand `~`/`$VAR`; refuse paths inside the binary install prefix; refuse paths whose parent is unwritable with a clear error
- [x] 3.5 Document precedence and examples in `README.md` and `CONTRIBUTING.md`

## 4. Relocate volatile artifacts

- [x] 4.1 Move backend PID file location from `data_dir/backend.pid` to `runtime_dir/backend.pid`; update `cleanup_orphaned_backend()` and `ProcessManager::new()` callers
- [x] 4.2 Move generated backend configs from `data_dir/generated/` to `runtime_dir/generated/`; update `ConfigWriter::new()` defaults; keep `backend.config_output_dir` user override
- [x] 4.3 Move geodata download dir and geodata index from `data_dir/geodata/` and `data_dir/geodata-index/` to `cache_dir/geodata/` and `cache_dir/geodata-index/`; update `GeodataManager`
- [x] 4.4 Move `latency_snapshot.json` from `data_dir` to `state_dir`; update `load_latency_snapshot`/`save_latency_snapshot`
- [x] 4.5 Add a one-shot relocation step in `AppPaths::ensure_dirs()` that moves legacy files into the new locations when the new locations are empty; log every move and continue on failure

## 5. Instance stamp + schema guard

- [x] 5.1 Define `InstanceStamp { profile, app_id, build_version, schema_version, first_started_at, last_started_at, last_started_pid }` and constants `CURRENT_SCHEMA_VERSION` + `BUILD_VERSION` (from `env!("CARGO_PKG_VERSION")`) in `crates/core/src/instance.rs`
- [x] 5.2 Implement `InstanceStamp::load_or_create(paths)` and `update_started(pid)` with atomic writes
- [x] 5.3 Implement `InstanceStamp::check_compatibility()` returning `Match | NeedsForwardMigration | IncompatibleProfile | IncompatibleAppId | TooNew`
- [x] 5.4 Wire the check into `try_run`: on `IncompatibleProfile`/`IncompatibleAppId`/`TooNew`, print actionable error and exit non-zero; on `NeedsForwardMigration`, run the existing migration entry points then bump the stamp
- [x] 5.5 Implement `--reset-instance`: wipes config/data/cache/runtime/state for the active profile after confirming the profile is non-production, or requiring `--i-understand` for production
- [x] 5.6 Unit-test all four compatibility branches and the reset flow

## 6. Single-instance lock

- [x] 6.1 Add `InstanceLock` in `crates/core/src/instance.rs` wrapping a `nix::fcntl::flock` exclusive lock on `runtime_dir/v2ray-rs.lock`
- [x] 6.2 Acquire the lock in `try_run` before any `ensure_dirs`/persistence work; on contention, print the holder PID from the stamp and exit with code 75
- [x] 6.3 Hold the lock for the lifetime of the process; release on `Drop`
- [x] 6.4 Add an integration test that spawns two child processes pointed at the same `--runtime-dir` and asserts the second exits with code 75

## 7. Tray + icon install respects profile

- [x] 7.1 Replace the `APP_ID`/`APP_ID_DEV` constants in `crates/ui/src/app.rs` with `profile.app_id()`
- [x] 7.2 Skip `install_icon_for_compositor` and `v2ray_rs_tray::install_icons` for non-production profiles unless `--install-icons` (or env) is set
- [x] 7.3 Update `crates/tray/src/icons.rs::data_dir()` to accept the resolved `AppPaths::data_dir()` instead of reading `XDG_DATA_HOME` directly
- [x] 7.4 Manual smoke check that dev and prod tray entries can coexist when both are explicitly enabled

## 8. Test ergonomics

- [x] 8.1 Update existing tests in `crates/core/src/persistence/*` and `crates/process/tests/*` to use `AppPaths::for_profile_in(AppProfile::Test, tmp.path())` instead of `from_paths`
- [x] 8.2 Drop the `cfg(any(test, feature = "test-utils"))` gate on `from_paths` (deprecated shim only)
- [x] 8.3 Add a `tests/path_isolation.rs` integration test asserting that `AppProfile::Production`, `Development`, and `Test` resolve to disjoint qualifier directories
- [x] 8.4 Add a CI smoke test that runs the binary with `--profile test --reset-instance` against a `TempDir` to confirm it starts cleanly

## 9. Spec alignment + docs

- [x] 9.1 Update delta specs (this change set): `app-persistence`, `process-lifecycle`, `geodata-management`, `config-generator`, `runtime-profiles`
- [x] 9.2 Add a "Runtime profiles" section to `README.md` with the override/profile table
- [x] 9.3 Add a "Local development profiles" section to `CONTRIBUTING.md` with `cargo run -- --profile development` recipes
- [x] 9.4 Update `CHANGELOG.md` with a `Changed` entry for the relocation and an `Added` entry for profiles + overrides
- [x] 9.5 Run `openspec validate runtime-profiles-and-path-overrides --strict` and `cargo test --workspace --all-targets` before archive
