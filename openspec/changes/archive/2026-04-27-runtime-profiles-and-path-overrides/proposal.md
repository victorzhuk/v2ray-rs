# Proposal: Runtime Profiles and Path Overrides

## Why
Today the app has only two storage slots: production (`ProjectDirs` qualifier `v2ray-rs`) and a single dev fallback (`v2ray-rs-dev`) selected by the `V2RAY_RS_DEV` env var. Everything else — the backend PID file, generated backend configs, geodata caches, latency snapshots, custom presets — is forced into one `data_dir`, and the system tray installs icons under a fixed App ID. This causes real problems for contributors and users:

- **Conflicts with production data.** Running `cargo run --release` from a working tree, or shipping a debug build to a tester, can write into the same `~/.config/v2ray-rs` and `~/.local/share/v2ray-rs` that the user's production install relies on. Schema or option drift between an in-progress build and the production build then corrupts production state.
- **No isolated test slot.** Integration tests rely on `AppPaths::from_paths()` (gated by a `test-utils` feature) but ad‑hoc local runs and packaged QA builds have no equivalent. There is no way to ask the binary itself to use a throwaway profile.
- **Volatile and durable data are mixed.** The backend PID file, generated `xray.json`/`sing-box.json`, and orphan-detection state live in `data_dir` next to user-owned subscriptions and presets. There is no `cache_dir`/`runtime_dir`/`state_dir` distinction, so cleanup, packaging (e.g. Flatpak), and multi-instance separation are all harder than they need to be.
- **Old builds running against new on-disk format silently misbehave.** There is no build/schema stamp; an older binary loaded against a newer-format store can persist back partial data and lose user state. The `repo-stabilization-and-spec-alignment` change already had to migrate persisted enum values once and the project will keep accumulating these migrations.
- **Single-instance is implicit.** PID-file orphan recovery exists for the backend, but there is no app-level lock per profile, so two parallel app instances of the same profile race on writes.
- **Tray + icon install is shared.** Both the development and production builds write into the user's `XDG_DATA_HOME/icons/hicolor` tree under different App IDs, but only the App ID toggles — there is no way to opt out for ephemeral profiles.

We need a flexible, runtime-configurable system where every directory and stamp is profile-scoped, can be overridden individually for tests/CI/QA, and refuses to load incompatible state instead of corrupting it.

## What Changes

- **Introduce `AppProfile`.** `Production`, `Development`, `Test`, `Custom { name }`. Profile is resolved from (in order) `--profile` CLI flag, `V2RAY_RS_PROFILE` env, `V2RAY_RS_DEV` legacy env (mapped to `Development`), and finally compile-time default (`Development` for debug builds, `Production` for release).
- **Extend `AppPaths` with full XDG layout.** Add `cache_dir`, `runtime_dir`, and `state_dir` accessors backed by `directories::ProjectDirs`/`BaseDirs`, with a profile-derived qualifier so each profile gets its own slot.
- **Relocate volatile artifacts.** PID files and generated backend configs move to `runtime_dir` (falling back to `data_dir` if `XDG_RUNTIME_DIR` is unavailable). Geodata files and the geodata index move to `cache_dir`. The latency snapshot moves to `state_dir`. User-authored data (settings, subscriptions, routing rules, custom presets, manual nodes) stays in `config_dir`/`data_dir`.
- **Per-path overrides.** Add CLI flags `--config-dir`, `--data-dir`, `--cache-dir`, `--runtime-dir`, `--state-dir`, and matching `V2RAY_RS_*_DIR` env vars. Each override replaces only that directory; unspecified ones still come from the resolved profile.
- **Profile-stamped guard file.** Write `<state_dir>/instance.json` containing `{ profile, app_id, build_version, schema_version, last_started_at }` on first launch. On every subsequent launch, refuse to start if `profile`, `app_id`, or `schema_version` mismatch the running build; show a clear error pointing the user at the override flags or a documented migration step. Production never falls back to "open anyway"; non-production profiles allow `--reset-instance` to wipe and start fresh.
- **Profile-scoped single-instance lock.** Acquire an advisory `flock` on `<runtime_dir>/v2ray-rs.lock` at startup. Same profile → second instance refuses to start (or attaches to the existing tray, out of scope here). Different profile → both instances run side-by-side without contention.
- **Tray and icon install become profile-aware.** App ID is `com.github.v2ray-rs[.<profile-suffix>]`, and non-production profiles default to *not* writing into the user's shared `XDG_DATA_HOME/icons/hicolor`; an explicit `--install-icons` flag (or env) re-enables it.
- **Test ergonomics.** `cargo test` integration tests default to `AppProfile::Test` rooted in a `TempDir`, with `AppPaths::for_profile_in(profile, root)` as the public seam (replacing the current `cfg(any(test, feature = "test-utils"))` constructor). Tests no longer need the feature flag to construct paths.
- **Documentation.** README and `CONTRIBUTING.md` get a "Runtime profiles" section listing the resolution order, the per-path overrides, and the cleanup commands for each profile.

## Capabilities

### New Capabilities
- `runtime-profiles`

### Modified Capabilities
- `app-persistence`
- `process-lifecycle`
- `geodata-management`
- `config-generator`
