## Purpose

Defines persistence of application settings, latency snapshots, and TUN configuration to disk, including forward-compatible deserialization from older `settings.toml` and snapshot formats.

## Requirements

### Requirement: Latency snapshot includes Real Delay samples
The system SHALL extend the persisted latency snapshot (under `state_dir/`) so that each entry can carry an optional `last_real_delay_ms` value in addition to the existing TCP sample. The serialized format SHALL be forward-compatible: a snapshot written by an older build SHALL load without error, with the missing Real Delay field defaulting to `None`; a snapshot written by a newer build that contains Real Delay values SHALL be ignored by older builds without preventing startup.

#### Scenario: Read legacy snapshot without Real Delay field
- **WHEN** the app loads a `latency_snapshot.json` written by a pre-Real-Delay build
- **THEN** all entries SHALL deserialize successfully with `last_real_delay_ms = None` and SHALL NOT trigger a parse error

#### Scenario: Write and reload snapshot with Real Delay field
- **WHEN** a Real Delay probe records `412` ms for a node, the snapshot is saved, and the app restarts
- **THEN** the reloaded snapshot SHALL contain `last_real_delay_ms = 412` for that node and the TCP sample SHALL remain unchanged

#### Scenario: Atomic write semantics preserved
- **WHEN** the latency snapshot is updated with Real Delay results
- **THEN** the write SHALL use the existing atomic-write path (temp file then rename) and SHALL NOT corrupt the snapshot if the app is killed mid-write

### Requirement: Real Delay settings persistence
The system SHALL persist Real Delay user preferences in `settings.toml` as a `[real_delay]` section with keys `enabled` (bool), `test_url` (string), `timeout_ms` (u32), and `use_for_lowest_latency` (bool). When the section is missing from a previously written `settings.toml`, the system SHALL fall back to the documented defaults without prompting the user.

#### Scenario: Missing real_delay section in legacy settings
- **WHEN** an existing `settings.toml` does not contain a `[real_delay]` section
- **THEN** the system SHALL load settings successfully with `enabled = true`, `test_url = "https://www.gstatic.com/generate_204"`, `timeout_ms = 5000`, `use_for_lowest_latency = false`, and SHALL NOT log an error

#### Scenario: Round-trip Real Delay settings
- **WHEN** the user changes the Real Delay settings (e.g. test URL and timeout), they are saved, and the app restarts
- **THEN** the reloaded `AppSettings` SHALL contain the same `[real_delay]` values

### Requirement: TUN configuration persistence
The system SHALL persist TUN preferences in `settings.toml` as a `[tun]` section
containing `enabled` (bool), `interface_name` (string), `mtu` (u16), `address_v4`
(CIDR string), optional `address_v6` (CIDR string), `stack` (enum), `strict_route`
(bool), `dns_hijack` (enum), `exclude_routes` (list of CIDR strings),
`exclude_processes` (list of process-name strings), and `exclude_domains` (list of
domain-suffix strings). When the section is absent from a previously written
`settings.toml`, the system SHALL load with documented defaults (TUN disabled, all
exclusion lists empty) without prompting or erroring.

#### Scenario: Missing tun section in legacy settings
- **WHEN** an existing `settings.toml` has no `[tun]` section
- **THEN** the system SHALL load successfully with `enabled = false`, the documented field defaults, and empty `exclude_routes`, `exclude_processes`, and `exclude_domains`, and SHALL NOT log an error

#### Scenario: Round-trip TUN settings
- **WHEN** the user changes TUN settings (e.g. interface name and MTU), they are saved, and the app restarts
- **THEN** the reloaded `AppSettings` SHALL contain the same `[tun]` values

#### Scenario: Exclusion lists round-trip
- **WHEN** the user sets `exclude_processes = ["cloudflared"]` and `exclude_domains = ["example.com"]`, the settings are saved, and the app restarts
- **THEN** the reloaded `AppSettings.tun` SHALL contain the same exclusion lists alongside the existing `exclude_routes`

#### Scenario: Forward-compatible deserialization
- **WHEN** a `settings.toml` written by a newer build contains TUN fields an older build does not recognize, or a `[tun]` section written before this change that lacks `exclude_processes`/`exclude_domains`
- **THEN** the build SHALL load the recognized fields, default the missing exclusion lists to empty, ignore unknown ones, and SHALL NOT fail startup

### Requirement: Logging settings persistence
The system SHALL persist backend logging preferences in `settings.toml` as a `[logging]` section with `backend_level` (one of `error`, `warning`, `info`, `debug`) and `connection_log` (bool). When the section or a key is absent, the system SHALL load `backend_level = "warning"` and `connection_log = false` without prompting or erroring. Changing either value while connected SHALL count as a restart-relevant change.

#### Scenario: Legacy settings without the section
- **WHEN** an existing `settings.toml` has no `[logging]` section
- **THEN** settings SHALL load with `backend_level = "warning"` and `connection_log = false` and SHALL NOT log an error

#### Scenario: Round-trip
- **WHEN** the user sets the level to `debug` and turns the connection log on, and the app restarts
- **THEN** the reloaded settings SHALL contain `backend_level = "debug"` and `connection_log = true`

#### Scenario: Change while connected
- **WHEN** the user changes the backend log level while connected
- **THEN** the pending-restart indication SHALL appear as for other config-changing settings

### Requirement: Versioned TUN ownership journal
The system SHALL write `tun_session.v1.json` atomically with owner-only access. It SHALL record version, session identity, boot and network namespace identity, controller process identity, device and reservation indices, phase, observed route/rule objects, and pending addition intent. A different application run SHALL NOT resume unfinished state without recovery.

#### Scenario: Mutation is interrupted
- **WHEN** the application or helper exits after recording intent
- **THEN** pending intent SHALL survive reload and SHALL NOT be assumed complete

#### Scenario: Controller identity changed
- **WHEN** a different application process tries to reuse unfinished recovery state
- **THEN** preparation SHALL refuse without replacing the prior journal

### Requirement: Unsafe journal reads fail closed
Journal reads SHALL be bounded and SHALL refuse symlinks, nonregular files, permissive modes, and ownership mismatches. Unsupported versions, malformed records, epoch mismatches with outstanding ownership, and failed recovery SHALL retain the original journal and refuse automatic mutation.

#### Scenario: Boot or namespace differs
- **WHEN** outstanding ownership or pending intent belongs to another boot or network namespace
- **THEN** automatic mutation SHALL be refused and the journal SHALL remain intact

#### Scenario: Fresh start after a finished session from another epoch
- **WHEN** a valid journal from another boot or namespace records Complete with no resources or pending intent
- **THEN** session preparation SHALL replace the journal with a fresh session identity without changing kernel state

#### Scenario: Fresh start after an unstarted sing-box session
- **WHEN** a valid sing-box Prepared journal has no device index, reservation index, resources, or pending intent
- **THEN** session preparation SHALL replace that journal without kernel cleanup, even after a boot or namespace change

#### Scenario: Journal cannot be safely interpreted
- **WHEN** a journal is malformed or from an unsupported version
- **THEN** the system SHALL report pending recovery without deleting or overwriting it

### Requirement: Legacy recovery evidence survives migration
The legacy `tun_session.json` SHALL remain diagnostic evidence and SHALL NOT be silently converted into kernel ownership. A pending legacy record SHALL block new sessions. Rollback to an older release SHALL require verified network cleanup first.

#### Scenario: Legacy marker remains intact
- **WHEN** a legacy marker is present
- **THEN** preparation SHALL refuse and the marker SHALL remain byte-identical until explicit recovery

### Requirement: Preserve generated configuration on routing load failure
When saved routing rules cannot be loaded, regeneration SHALL fail with a reported error and SHALL leave the existing generated config file byte-identical. The system SHALL NOT write the file and SHALL NOT substitute a default-empty rule set.

#### Scenario: Routing rules fail to load
- **WHEN** regeneration runs while the saved routing rules cannot be loaded
- **THEN** regeneration SHALL report the error and the existing generated config file SHALL remain byte-identical

### Requirement: Non-ASCII preset slugs, distinct non-zero ports, and serialized flushes
Custom preset filenames SHALL append a fixed cross-release deterministic 64-bit hash suffix (computed over the UTF-8 preset name without `DefaultHasher`) when a preset name contains no ASCII alphanumeric characters, `AppSettings` validation SHALL reject port `0` or `socks_port == http_port`, pending debounced settings saves SHALL flush immediately on Preferences close and application quit, background subscription saves SHALL serialize with reloads, and subscription payload decoding/parsing SHALL run on `spawn_blocking`.

#### Scenario: Distinct Cyrillic preset names produce distinct files
- **WHEN** the user saves two custom routing presets named `"Работа"` and `"Дом"`
- **THEN** both presets SHALL persist to fixed, distinct `preset-<16hex>.json` filenames derived from a deterministic non-`DefaultHasher` digest without overwriting each other or producing `".json"`, and saving the same name again SHALL overwrite its existing file across Rust toolchain upgrades

#### Scenario: Closing Preferences or quitting flushes pending debounced edits
- **WHEN** the user edits a setting and closes Preferences or quits within the 300 ms debounce window
- **THEN** the pending `AppSettings` state SHALL be written to `settings.toml` before close or exit completes

### Requirement: Pasted subscription blobs write atomically with owner-only permissions
When saving pasted subscription or configuration-bundle content under `subscription_blobs/`, the system SHALL create and write the file atomically with owner-only `0600` permissions from file creation rather than writing with default umask permissions before changing mode.

#### Scenario: Pasted JSON bundle has mode 0600 throughout creation
- **WHEN** a user imports a subscription from pasted JSON content
- **THEN** the persisted blob under `subscription_blobs/` SHALL be written atomically with mode `0600` and SHALL NOT pass through a group-readable or world-readable state
