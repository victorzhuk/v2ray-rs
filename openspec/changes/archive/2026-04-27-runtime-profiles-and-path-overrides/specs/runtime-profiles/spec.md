# Spec Delta: runtime-profiles

## ADDED Requirements

### Requirement: Profile selection
The system SHALL select an `AppProfile` (`Production`, `Development`, `Test`, or `Custom(name)`) at startup using the resolution order: `--profile` CLI flag > `V2RAY_RS_PROFILE` env > legacy `V2RAY_RS_DEV` env > compile-time default (`Development` for debug builds, `Production` for release builds).

#### Scenario: Default release build
- **WHEN** the binary is built in release mode and started with no CLI flags or env overrides
- **THEN** the system SHALL resolve the profile to `Production`

#### Scenario: Default debug build
- **WHEN** the binary is built in debug mode and started with no CLI flags or env overrides
- **THEN** the system SHALL resolve the profile to `Development`

#### Scenario: CLI flag wins over env
- **WHEN** the binary is started with `--profile test` and `V2RAY_RS_PROFILE=production`
- **THEN** the system SHALL resolve the profile to `Test`

#### Scenario: Legacy env still maps to development
- **WHEN** the binary is started with `V2RAY_RS_DEV=1` and no other profile signal
- **THEN** the system SHALL resolve the profile to `Development` and SHALL log a deprecation warning naming the replacement env var

#### Scenario: Invalid custom profile name is rejected
- **WHEN** the binary is started with `--profile "Bad Name!"` or any name not matching `[a-z0-9][a-z0-9_-]{0,30}`
- **THEN** the system SHALL refuse to start and SHALL print a message naming the allowed character set

### Requirement: Profile-scoped App ID and qualifier
The system SHALL derive both the on-disk directory qualifier and the desktop App ID from the active profile so that different profiles never share storage or tray entries.

#### Scenario: Production qualifier
- **WHEN** the active profile is `Production`
- **THEN** the storage qualifier SHALL be `v2ray-rs` and the App ID SHALL be `com.github.v2ray-rs`

#### Scenario: Development qualifier
- **WHEN** the active profile is `Development`
- **THEN** the storage qualifier SHALL be `v2ray-rs-dev` and the App ID SHALL be `com.github.v2ray-rs.dev`

#### Scenario: Custom profile qualifier
- **WHEN** the active profile is `Custom("qa")`
- **THEN** the storage qualifier SHALL be `v2ray-rs-qa` and the App ID SHALL be `com.github.v2ray-rs.qa`

### Requirement: Per-directory path overrides
The system SHALL allow each of `config_dir`, `data_dir`, `cache_dir`, `runtime_dir`, and `state_dir` to be overridden independently. Per-directory CLI flags take precedence over per-directory env vars, which take precedence over profile-derived XDG defaults. Unspecified directories SHALL still be derived from the active profile.

#### Scenario: Override only the runtime directory
- **WHEN** the binary is started with `--runtime-dir /tmp/v2ray-rs-runtime` and no other overrides
- **THEN** the runtime directory SHALL be `/tmp/v2ray-rs-runtime` and config, data, cache, and state directories SHALL retain their profile-derived defaults

#### Scenario: Env var overrides default but loses to CLI flag
- **WHEN** the binary is started with `V2RAY_RS_DATA_DIR=/srv/a` and `--data-dir /srv/b`
- **THEN** the data directory SHALL be `/srv/b`

#### Scenario: Reject relative override paths
- **WHEN** the binary is started with `--config-dir ./relative/path`
- **THEN** the system SHALL refuse to start and SHALL print a message requiring an absolute path or one starting with `~` / `$VAR`

### Requirement: Tray and icon install opt-out for non-production
The system SHALL NOT install icons into the user's shared `XDG_DATA_HOME/icons/hicolor` for non-production profiles unless `--install-icons` or `V2RAY_RS_INSTALL_ICONS=1` is set.

#### Scenario: Development build does not pollute shared icon theme
- **WHEN** the active profile is `Development` and no install-icons flag is set
- **THEN** the system SHALL NOT write any files under `XDG_DATA_HOME/icons/`

#### Scenario: Development build with explicit opt-in
- **WHEN** the active profile is `Development` and `--install-icons` is set
- **THEN** the system SHALL install icons under the App ID `com.github.v2ray-rs.dev`
