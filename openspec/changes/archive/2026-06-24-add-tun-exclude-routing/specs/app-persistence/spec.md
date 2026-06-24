## MODIFIED Requirements

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
