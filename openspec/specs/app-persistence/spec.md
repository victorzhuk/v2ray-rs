## ADDED Requirements

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
The system SHALL persist TUN preferences in `settings.toml` as a `[tun]` section containing `enabled` (bool), `interface_name` (string), `mtu` (u16), `address_v4` (CIDR string), optional `address_v6` (CIDR string), `stack` (enum), `strict_route` (bool), `dns_hijack` (enum), and `exclude_routes` (list of CIDR strings). When the section is absent from a previously written `settings.toml`, the system SHALL load with documented defaults (TUN disabled) without prompting or erroring.

#### Scenario: Missing tun section in legacy settings
- **WHEN** an existing `settings.toml` has no `[tun]` section
- **THEN** the system SHALL load successfully with `enabled = false` and the documented field defaults, and SHALL NOT log an error

#### Scenario: Round-trip TUN settings
- **WHEN** the user changes TUN settings (e.g. interface name and MTU), they are saved, and the app restarts
- **THEN** the reloaded `AppSettings` SHALL contain the same `[tun]` values

#### Scenario: Forward-compatible deserialization
- **WHEN** a `settings.toml` written by a newer build contains TUN fields an older build does not recognize
- **THEN** the older build SHALL load the recognized fields, ignore the unknown ones, and SHALL NOT fail startup
