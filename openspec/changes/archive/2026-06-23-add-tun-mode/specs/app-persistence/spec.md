## ADDED Requirements

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
