# Spec: Runtime Profiles

## Purpose

Support selectable runtime profiles with isolated directories, App ID/qualifier, per-directory overrides, and non-production icon install opt-out.
## Requirements
### Requirement: Profile selection
The system SHALL select an `AppProfile` (`Production`, `Development`, `Test`, or `Custom(name)`) at startup using resolution order: `--profile` CLI flag > `V2RAY_RS_PROFILE` env > legacy `V2RAY_RS_DEV` env > compile-time default (`Development` for debug builds, `Production` for release builds).

#### Scenario: Default release build
- **WHEN** binary is built in release mode and started with no CLI flags or env overrides
- **THEN** system SHALL resolve profile to `Production`

#### Scenario: Default debug build
- **WHEN** binary is built in debug mode and started with no CLI flags or env overrides
- **THEN** system SHALL resolve profile to `Development`

#### Scenario: CLI flag wins over env
- **WHEN** binary is started with `--profile test` and `V2RAY_RS_PROFILE=production`
- **THEN** system SHALL resolve profile to `Test`

#### Scenario: Legacy env still maps to development
- **WHEN** binary is started with `V2RAY_RS_DEV=1` and no other profile signal
- **THEN** system SHALL resolve profile to `Development` and SHALL log a deprecation warning naming replacement env var

#### Scenario: Invalid custom profile name is rejected
- **WHEN** binary is started with `--profile "Bad Name!"` or any name not matching `[a-z0-9][a-z0-9_-]{0,30}`
- **THEN** system SHALL refuse to start and SHALL print a message naming allowed character set

### Requirement: Profile-scoped App ID and qualifier
The system SHALL derive both on-disk directory qualifier and desktop App ID from active profile so that different profiles never share storage or tray entries.

#### Scenario: Production qualifier
- **WHEN** active profile is `Production`
- **THEN** storage qualifier SHALL be `v2ray-rs` and App ID SHALL be `com.github.v2ray-rs`

#### Scenario: Development qualifier
- **WHEN** active profile is `Development`
- **THEN** storage qualifier SHALL be `v2ray-rs-dev` and App ID SHALL be `com.github.v2ray-rs.dev`

#### Scenario: Custom profile qualifier
- **WHEN** active profile is `Custom("qa")`
- **THEN** storage qualifier SHALL be `v2ray-rs-qa` and App ID SHALL be `com.github.v2ray-rs.qa`

### Requirement: Per-directory path overrides
The system SHALL allow each of `config_dir`, `data_dir`, `cache_dir`, `runtime_dir`, and `state_dir` to be overridden independently. Per-directory CLI flags take precedence over per-directory env vars, which take precedence over profile-derived XDG defaults. Unspecified directories SHALL still be derived from active profile.

#### Scenario: Override only runtime directory
- **WHEN** binary is started with `--runtime-dir /tmp/v2ray-rs-runtime` and no other overrides
- **THEN** runtime directory SHALL be `/tmp/v2ray-rs-runtime` and config, data, cache, and state directories SHALL retain their profile-derived defaults

#### Scenario: Env var overrides default but loses to CLI flag
- **WHEN** binary is started with `V2RAY_RS_DATA_DIR=/srv/a` and `--data-dir /srv/b`
- **THEN** data directory SHALL be `/srv/b`

#### Scenario: Reject relative override paths
- **WHEN** binary is started with `--config-dir ./relative/path`
- **THEN** system SHALL refuse to start and SHALL print a message requiring an absolute path or one starting with `~` / `$VAR`

### Requirement: Tray and icon install opt-out for non-production
The system SHALL NOT install icons into user's shared `XDG_DATA_HOME/icons/hicolor` for non-production profiles unless `--install-icons` or `V2RAY_RS_INSTALL_ICONS=1` is set.

#### Scenario: Development build does not pollute shared icon theme
- **WHEN** active profile is `Development` and no install-icons flag is set
- **THEN** system SHALL NOT write any files under `XDG_DATA_HOME/icons/`

#### Scenario: Development build with explicit opt-in
- **WHEN** active profile is `Development` and `--install-icons` is set
- **THEN** system SHALL install icons under App ID `com.github.v2ray-rs.dev`

### Requirement: XDG runtime and state directory fallback
The system SHALL resolve the runtime and state directories through an injectable environment source. When `XDG_RUNTIME_DIR` is present, `runtime_dir` SHALL be that directory joined with the active profile's qualifier; when it is absent, `runtime_dir` SHALL fall back to `data_dir/runtime`. When `XDG_STATE_HOME` is present, `state_dir` SHALL be that directory joined with the qualifier; when it is absent, `state_dir` SHALL fall back to `data_dir/state`. Because resolution reads through the injected source rather than the process environment directly, the fallback SHALL be verifiable without mutating process-global environment variables.

#### Scenario: XDG_RUNTIME_DIR present
- **WHEN** the environment source reports a value for `XDG_RUNTIME_DIR`
- **THEN** `runtime_dir` SHALL be that value joined with the active profile's qualifier

#### Scenario: XDG_RUNTIME_DIR absent
- **WHEN** the environment source reports no value for `XDG_RUNTIME_DIR`
- **THEN** `runtime_dir` SHALL be `data_dir/runtime`

#### Scenario: XDG_STATE_HOME present
- **WHEN** the environment source reports a value for `XDG_STATE_HOME`
- **THEN** `state_dir` SHALL be that value joined with the active profile's qualifier

#### Scenario: XDG_STATE_HOME absent
- **WHEN** the environment source reports no value for `XDG_STATE_HOME`
- **THEN** `state_dir` SHALL be `data_dir/state`

#### Scenario: Fallback verified without touching process environment
- **WHEN** a test supplies an environment source that omits `XDG_RUNTIME_DIR` and `XDG_STATE_HOME`
- **THEN** the resolver SHALL return the `data_dir` fallbacks without reading or mutating the real process environment

### Requirement: Instance stamp stays accurate and legacy files are cleaned up
The system SHALL refresh `build_version` in the instance stamp on every start, and legacy-location files (pre-XDG-split `generated/`, `geodata/`, PID and snapshot files under `data_dir`) SHALL be migrated to their current locations with the destination directory created as needed. When the destination already holds current data, the legacy copies SHALL be deleted rather than retained; generated configs contain node credentials and MUST NOT linger in abandoned locations.

#### Scenario: Stamp reflects the running build
- **WHEN** the app starts with an instance stamp written by an older build
- **THEN** after startup the stamp's `build_version` SHALL equal the running build's version

#### Scenario: Relocation into a missing destination directory
- **WHEN** legacy `data_dir/generated/` files exist and `runtime_dir/generated/` does not yet exist
- **THEN** the relocation SHALL create the destination directory and move the files, leaving the legacy directory removed

#### Scenario: Populated destination still clears legacy copies
- **WHEN** legacy `data_dir/generated/` files exist and `runtime_dir/generated/` already contains current configs
- **THEN** the legacy files SHALL be deleted

