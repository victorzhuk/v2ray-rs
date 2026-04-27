# Design: Runtime Profiles and Path Overrides

## Context
`AppPaths` currently exposes only `config_dir()` and `data_dir()`, both keyed off `ProjectDirs::from("com", "v2ray-rs", "v2ray-rs"|"v2ray-rs-dev")`. Every other directory in the codebase is derived inside ad‑hoc helpers: tray icons read `XDG_DATA_HOME` directly, generated configs land in `data_dir/generated`, the backend PID file lands in `data_dir/backend.pid`, geodata in `data_dir/geodata`. The dev/prod toggle is the env var `V2RAY_RS_DEV`, checked once at startup in `crates/ui/src/app.rs::try_run`. There is no schema stamp, no app-level lock, and no per-path override.

This design extends the existing layout instead of replacing it: the persistence module remains the source of truth for paths, and we add structured profile selection plus per-directory overrides above it.

## Design Decisions

### 1. Profile is a first-class type, resolved once at startup
- `AppProfile { Production, Development, Test, Custom(String) }` lives in `v2ray-rs-core::profile`.
- Resolution order (first match wins):
  1. `--profile <name>` CLI flag
  2. `V2RAY_RS_PROFILE` env (`production`/`development`/`test`/anything-else=`custom:<name>`)
  3. `V2RAY_RS_DEV=1` legacy env → `Development` (kept for one release, logged as deprecated)
  4. Compile-time default: `Development` if `cfg!(debug_assertions)`, else `Production`
- Custom name is restricted to `[a-z0-9][a-z0-9_-]{0,30}` and rejected otherwise so it is safe to use in directory names and App IDs.
- Profile is stored on `AppPaths` so callers never re-resolve it and tests can construct any profile freely.

### 2. AppPaths exposes the full XDG set, not just config + data
- Add `cache_dir()`, `runtime_dir()`, `state_dir()` alongside the existing `config_dir()`/`data_dir()`.
- `runtime_dir()` is `XDG_RUNTIME_DIR/<qualifier>` when available, otherwise `data_dir().join("runtime")` (matching the spec's documented fallback).
- `state_dir()` uses `BaseDirs::state_dir()` (or `data_dir()/state` fallback for older Linux distros without one).
- The qualifier is derived from the profile so each profile gets its own slot:
  - `Production` → `v2ray-rs`
  - `Development` → `v2ray-rs-dev`
  - `Test` → `v2ray-rs-test` (only used when not rooted in a TempDir)
  - `Custom("qa")` → `v2ray-rs-qa`

### 3. Volatile vs durable: clear placement rules
| Artifact | Old location | New location | Rationale |
|---|---|---|---|
| `settings.toml` | `config_dir/` | unchanged | user-edited durable |
| `subscriptions.json`, `routing_rules.json`, `custom_nodes.json`, `presets/` | `data_dir/` | unchanged | user-owned durable |
| Generated `xray.json`/`v2ray.json`/`sing-box.json` | `data_dir/generated/` | `runtime_dir/generated/` | regenerated each launch |
| `backend.pid` | `data_dir/` | `runtime_dir/` | volatile, profile-local |
| `latency_snapshot.json` | `data_dir/` | `state_dir/` | derived metrics, not user input |
| `geodata/*.dat`, `geodata-index/` | `data_dir/geodata/` | `cache_dir/geodata/` | downloaded, regenerable |
| `instance.json` (new) | — | `state_dir/` | guard stamp, see §5 |
| `v2ray-rs.lock` (new) | — | `runtime_dir/` | single-instance lock |

The `backend.config_output_dir` user override in settings keeps working — it now overrides the runtime path instead of the data path. Existing user data is migrated on first launch (see §6).

### 4. Per-directory overrides have higher precedence than profile
- CLI flags: `--config-dir`, `--data-dir`, `--cache-dir`, `--runtime-dir`, `--state-dir`.
- Env vars: `V2RAY_RS_CONFIG_DIR`, `V2RAY_RS_DATA_DIR`, `V2RAY_RS_CACHE_DIR`, `V2RAY_RS_RUNTIME_DIR`, `V2RAY_RS_STATE_DIR`.
- Resolution per-directory: CLI > env > profile-derived XDG path. Unspecified directories still come from the profile. This lets CI override only the runtime/cache dirs while keeping config under version-controlled fixtures.
- Overrides apply to the *root* of each directory; sub-paths (e.g. `runtime_dir/generated/xray.json`) continue to be derived inside `AppPaths`.

### 5. instance.json is the compatibility stamp
On first launch a profile writes `state_dir/instance.json`:

```json
{
  "profile": "production",
  "app_id": "com.github.v2ray-rs",
  "build_version": "0.7.3",
  "schema_version": 4,
  "first_started_at": "2026-04-27T12:34:56Z",
  "last_started_at": "2026-04-27T12:34:56Z"
}
```

On every subsequent launch the running build compares `profile`, `app_id`, and `schema_version`:
- All match → update `last_started_at` and continue.
- `schema_version` is **lower** than current → run forward migrations (existing `RefMigration` flow), bump the file.
- `schema_version` is **higher** than current → an older binary is being run against a newer store. Refuse to start. Print:
  > "This profile was last used by a newer build (schema 5, this build supports up to 4). Either upgrade, or run `v2ray-rs --profile <name> --reset-instance` to wipe it."
- `profile` or `app_id` mismatch → refuse to start with the same kind of message.

`--reset-instance` is gated on non-production profiles by default, and on production it requires `--reset-instance --i-understand` to avoid accidental data loss.

### 6. One-time relocation migration
On the first launch where the new layout is detected to be empty *and* the old layout has files, move:
- `data_dir/backend.pid` → `runtime_dir/backend.pid`
- `data_dir/generated/*` → `runtime_dir/generated/*`
- `data_dir/geodata/*` → `cache_dir/geodata/*`
- `data_dir/latency_snapshot.json` → `state_dir/latency_snapshot.json`

The migration is best-effort (`std::fs::rename` then `copy+remove` fallback for cross-filesystem moves), logged, and writes `instance.json` on completion. If any step fails, the launch continues using the old paths and emits a warning so the user is not blocked.

### 7. Single-instance lock per profile
- `<runtime_dir>/v2ray-rs.lock` is `flock`-ed exclusively at startup using `nix::fcntl::flock`.
- Lock acquisition failure → the new instance prints which PID owns the lock (looked up via `instance.json::last_started_pid`) and exits with code 75 (`EX_TEMPFAIL`).
- Lock release happens on normal shutdown and on process exit (kernel releases the file lock automatically).
- Different profiles use different `runtime_dir`s and therefore never contend.

### 8. Tray icon install respects the profile
- App ID resolution moves into `core::profile::app_id_for(profile)` returning `com.github.v2ray-rs` / `com.github.v2ray-rs.dev` / `com.github.v2ray-rs.test` / `com.github.v2ray-rs.<custom>`.
- Icon installation into the user's `XDG_DATA_HOME/icons/hicolor` is skipped for non-production profiles unless `--install-icons` (or `V2RAY_RS_INSTALL_ICONS=1`) is set. The tray's own `icon_theme_path()` continues to use a profile-private temp dir, so the tray icon still appears.

### 9. Test ergonomics
- `AppPaths::for_profile_in(profile: AppProfile, root: &Path)` is the new public test seam, available without the `test-utils` feature.
- It assigns `config_dir = root/config`, `data_dir = root/data`, `cache_dir = root/cache`, `runtime_dir = root/runtime`, `state_dir = root/state`. No XDG lookup happens, no environment is read.
- Existing `AppPaths::from_paths(config, data)` is kept as a deprecated shim that delegates to `for_profile_in`.

## Trade-offs Considered

- **Single config knob vs. per-directory overrides.** A single `--root <path>` would be simpler but does not solve the common CI case of "use the repo's checked-in config but a temp data/cache dir." Per-directory overrides cost a little API surface and pay it back across every workflow.
- **Refuse vs. auto-migrate on schema mismatch.** Auto-downgrading a newer store to an older format would silently lose fields. Refuse-with-clear-message is the safer default; we already have a pattern for forward migrations and we extend that, never the reverse.
- **One lock vs. two.** A separate `app.lock` and `backend.lock` was considered. Collapsing to one app-level `v2ray-rs.lock` plus the existing backend PID file keeps the contracts simple: app-level prevents two GUIs, the PID file prevents two backends, and they live in the same profile-scoped `runtime_dir`.
- **Drop the `V2RAY_RS_DEV` env immediately vs. one-release deprecation.** Keeping the legacy env mapped to `Development` for one release avoids breaking contributor muscle memory. It is logged at WARN so it is visible and removable later.
