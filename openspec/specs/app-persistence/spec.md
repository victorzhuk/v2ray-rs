# Spec: app-persistence

## ADDED Requirements

### Requirement: XDG-compliant storage paths
The system SHALL store files according to the full XDG Base Directory Specification, scoped by the active profile's qualifier:
- Configuration in `$XDG_CONFIG_HOME/<qualifier>/` (default `~/.config/<qualifier>/`)
- Durable user data in `$XDG_DATA_HOME/<qualifier>/` (default `~/.local/share/<qualifier>/`)
- Regenerable caches in `$XDG_CACHE_HOME/<qualifier>/` (default `~/.cache/<qualifier>/`)
- Volatile runtime artifacts in `$XDG_RUNTIME_DIR/<qualifier>/` (falling back to `<data_dir>/runtime/` when unset)
- Derived state in `$XDG_STATE_HOME/<qualifier>/` (falling back to `<data_dir>/state/` when unset)

The qualifier SHALL be derived from the active profile (`v2ray-rs` for production, `v2ray-rs-dev` for development, `v2ray-rs-test` for test, `v2ray-rs-<name>` for custom profiles).

#### Scenario: First launch directory creation
- **WHEN** the app launches and resolved storage directories do not exist
- **THEN** system SHALL create config, data, cache, runtime, and state directories with `0o700` permissions

#### Scenario: XDG override
- **WHEN** any of `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_RUNTIME_DIR`, or `XDG_STATE_HOME` is set
- **THEN** system SHALL place that directory under the overridden root, joined with the profile qualifier

#### Scenario: Profile isolates storage
- **WHEN** same user launches the binary with `--profile production` and later with `--profile development`
- **THEN** the two runs SHALL read and write disjoint config, data, cache, runtime, and state directories with no cross-contamination

#### Scenario: Per-directory override
- **WHEN** user launches with `--data-dir /tmp/scratch/data`
- **THEN** durable user data SHALL be read from and written to `/tmp/scratch/data` while config, cache, runtime, and state directories remain at their profile-derived defaults

### Requirement: Settings persistence
The system SHALL serialize application settings to TOML format and store them in the config directory. The system SHALL deserialize settings on startup.

#### Scenario: Save and reload settings
- **WHEN** the user changes a setting and the app restarts
- **THEN** the changed setting SHALL be preserved

#### Scenario: Corrupt config handling
- **WHEN** the config file is corrupted or unparseable
- **THEN** the system SHALL fall back to defaults and warn the user

### Requirement: Data persistence
The system SHALL serialize subscriptions, proxy nodes, and routing rules to JSON format and store them in the data directory.

#### Scenario: Subscription data round-trip
- **WHEN** subscriptions are saved and reloaded
- **THEN** all subscription data including nodes and metadata SHALL be preserved

#### Scenario: Atomic writes
- **WHEN** data is saved to disk
- **THEN** the system SHALL use atomic write operations (write to temp file, then rename) to prevent data corruption on crash

### Requirement: Restore persisted runtime configuration
The system SHALL be able to restore persisted settings and routing rules to the last launched runtime snapshot when the user discards connected-state changes.

#### Scenario: Discard connected edits
- **WHEN** the user selects "Discard" from the restart-required banner
- **THEN** the system overwrites the current persisted settings and routing rules with the active runtime snapshot and clears the pending divergence state

### Requirement: Custom nodes persistence
The system SHALL persist manual proxy nodes in `custom_nodes.json` under the app data directory using atomic writes.

#### Scenario: Save and reload manual nodes
- **WHEN** manual nodes are saved and the app restarts
- **THEN** the nodes round-trip with their IDs, enabled state, and protocol data intact

#### Scenario: Corrupt custom nodes file
- **WHEN** `custom_nodes.json` is unreadable or invalid
- **THEN** the system falls back to an empty manual-node list and reports the problem without preventing startup

### Requirement: Restore launched manual nodes on discard
The system SHALL restore the launched manual-node set when the user discards connected-state manual-node changes.

#### Scenario: Discard connected manual-node edits
- **WHEN** the backend is connected, the user changes manual nodes, and then selects `Discard`
- **THEN** the system overwrites the current persisted manual-node set with the last launched manual-node snapshot and clears the pending divergence state

### Requirement: Volatile and durable artifacts are placed by purpose
The system SHALL place files according to their durability:
- User-authored durable data (settings, subscriptions, routing rules, manual nodes, custom presets) in `config_dir`/`data_dir`
- Regenerable caches (geodata files, geodata index) in `cache_dir`
- Volatile runtime artifacts (backend PID file, generated backend configs, instance lock) in `runtime_dir`
- Derived state (latency snapshots, instance stamp) in `state_dir`

#### Scenario: PID file lives in runtime dir
- **WHEN** the backend is launched
- **THEN** the backend PID file SHALL be written to `runtime_dir/backend.pid` and SHALL NOT appear in `data_dir`

#### Scenario: Geodata lives in cache dir
- **WHEN** geodata is downloaded
- **THEN** geodata files and the geodata index SHALL be written under `cache_dir/geodata/` and `cache_dir/geodata-index/`

### Requirement: Instance compatibility stamp
The system SHALL write `instance.json` to `state_dir/` containing the active profile, App ID, build version, and persistence schema version. On every launch the running build SHALL compare the stamp against itself and refuse to start when the stamp records an incompatible profile, App ID, or a `schema_version` higher than the running build supports.

#### Scenario: First launch writes stamp
- **WHEN** the app launches and `state_dir/instance.json` does not exist
- **THEN** the system SHALL create it with the active profile, App ID, build version, and current schema version, then continue startup

#### Scenario: Newer store refuses to load in older build
- **WHEN** the stamp records `schema_version` higher than the running build supports
- **THEN** the system SHALL refuse to start and SHALL print a message naming the recorded schema, the supported schema, and the `--reset-instance` recovery option

#### Scenario: Profile mismatch is refused
- **WHEN** the stamp records a profile or App ID different from the active one
- **THEN** the system SHALL refuse to start and SHALL print a message identifying the conflict

#### Scenario: Forward migration on lower schema version
- **WHEN** the stamp records a `schema_version` lower than the running build supports
- **THEN** the system SHALL run forward migrations and SHALL update the stamp to the current schema version

### Requirement: Reset instance command
The system SHALL support `--reset-instance` to wipe a profile's config, data, cache, runtime, and state directories. For non-production profiles the flag SHALL be sufficient. For the production profile the flag SHALL additionally require `--i-understand` to avoid accidental data loss.

#### Scenario: Reset a development profile
- **WHEN** the binary is started with `--profile development --reset-instance`
- **THEN** the system SHALL remove the development profile's directories and start fresh

#### Scenario: Reset production requires explicit confirmation
- **WHEN** the binary is started with `--profile production --reset-instance` without `--i-understand`
- **THEN** the system SHALL refuse to wipe data and SHALL print the additional flag required to proceed

### Requirement: One-time relocation of legacy artifacts
The system SHALL detect a pre-relocation layout and move legacy files to their new locations on first launch. Relocation failures SHALL be logged but SHALL NOT block startup.

#### Scenario: Migrate legacy PID and generated configs
- **WHEN** the legacy `data_dir/backend.pid` or `data_dir/generated/*.json` exist and the new locations are empty
- **THEN** the system SHALL move those files into `runtime_dir/` and SHALL log each relocation

#### Scenario: Migrate legacy geodata
- **WHEN** legacy `data_dir/geodata/` files exist and `cache_dir/geodata/` is empty
- **THEN** the system SHALL move geodata files and the geodata index into `cache_dir/`

#### Scenario: Relocation failure is non-fatal
- **WHEN** any relocation step fails (e.g. cross-filesystem rename without copy fallback success)
- **THEN** the system SHALL log the failure and SHALL continue startup using the old paths
