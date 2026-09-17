## ADDED Requirements

### Requirement: Partial DNS settings load without data loss
Loading `settings.toml` SHALL accept a `[dns]` table that omits `enabled`, treating it as `false`, so a partial table never makes the whole settings file unreadable. An explicitly present `servers` key SHALL be loaded as written, including an empty list. The two default servers SHALL be supplied only when the `servers` key is absent and no legacy `remote`/`domestic` fields are present.

#### Scenario: Missing enabled key
- **WHEN** `settings.toml` contains `[dns]` with `servers` but no `enabled` key, and other sections carry non-default values
- **THEN** settings SHALL load with `dns.enabled = false`, the configured servers, and every other section's values preserved

#### Scenario: Cleared server list survives reload
- **WHEN** DNS is disabled, the user removes every DNS server, and settings are saved and loaded again
- **THEN** the loaded `dns.servers` SHALL be empty

#### Scenario: Missing servers key gets defaults
- **WHEN** `settings.toml` contains `[dns]` with `enabled = false` and neither `servers` nor legacy `remote`/`domestic`
- **THEN** the loaded `dns.servers` SHALL be the two default servers

### Requirement: Downgraded DNS servers use the DoH default port
When a DNS server's protocol is downgraded to DoH for the selected backend, the generated address SHALL use DoH's default port and SHALL NOT carry a port set for the original protocol. A server configured as DoH keeps its explicit port.

#### Scenario: DoT with explicit port on v2ray
- **WHEN** the backend is v2ray and a server is DoT `dns.google` with port 853
- **THEN** the generated address SHALL be `https://dns.google/dns-query`

#### Scenario: Native DoH keeps its port
- **WHEN** a server is DoH `doh.example.com` with port 8443
- **THEN** the generated address SHALL be `https://doh.example.com:8443/dns-query`