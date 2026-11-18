# Spec Delta

## MODIFIED Requirements

### Requirement: Named DNS servers
The system SHALL support multiple named DNS servers. Each server SHALL have a tag (unique string identifier), protocol, address, optional port, and optional detour (outbound tag for routing DNS traffic).

#### Scenario: Default DNS servers
- **WHEN** no DNS configuration exists (fresh install)
- **THEN** the system SHALL provide two default servers: a `"remote"` server (DoH, `1.1.1.1`, detour `"proxy"`) and a `"domestic"` server (UDP, `223.5.5.5`, detour `"direct"`)

#### Scenario: Add custom server
- **WHEN** the user adds a DNS server with tag "adguard", protocol DoH, address "dns.adguard.com"
- **THEN** the server SHALL be persisted and available for DNS rule assignment

#### Scenario: Duplicate tag rejected
- **WHEN** the user attempts to add a server with a tag that already exists
- **THEN** the system SHALL reject the addition with a validation error

### Requirement: Backward-compatible deserialization with migration
The system SHALL deserialize existing settings.toml files (with the old minimal DnsConfig format) without error, migrating old field values to the new model.

#### Scenario: Old config migrates server addresses
- **WHEN** a settings.toml contains `[dns]` with `enabled = true`, `remote = { protocol = "doh", address = "8.8.8.8" }`, and `domestic = { protocol = "plain", address = "114.114.114.114" }`
- **THEN** the system SHALL load successfully with `enabled = true`, `servers` containing a `"remote"` server (DoH, `8.8.8.8`, detour `"proxy"`) and a `"domestic"` server (UDP, `114.114.114.114`, detour `"direct"`), and all new fields at defaults

#### Scenario: Old config does not lose user DNS settings
- **WHEN** a settings.toml has old-format `remote` and `domestic` fields with non-default addresses
- **THEN** those addresses SHALL be preserved in the migrated `servers` list, NOT replaced by defaults

#### Scenario: Fresh config with no dns section
- **WHEN** a settings.toml has no `[dns]` section at all
- **THEN** the system SHALL use `DnsConfig::default()` with DNS disabled and two default servers

#### Scenario: New format loads directly
- **WHEN** a settings.toml contains `[dns]` with `servers` array in the new format
- **THEN** the system SHALL load the new format directly without migration

### Requirement: Effective DNS resolver set is derivable
The system SHALL derive, for a backend, the DNS settings, the TUN settings, and the routing rules used to generate a config, the ordered list of resolvers the generated config uses. Each entry SHALL state the resolver address, its transport, its path (`direct`, `proxy`, `routing` when the backend's routing rules decide, `system` for the operating-system resolver, or `static` for host overrides), its source (`user`, `fallback`, `bootstrap`, `system`, or `profile` when a subscription's imported profile supplied the DNS settings), and its scope (all domains or the listed domains). Static host override entries SHALL include only override mappings that satisfy the backend's active query strategy so domains omitted from the generated `hosts` table never appear in the Effective DNS list. The list SHALL match the resolvers present in the generated config for the same inputs. Deriving it SHALL NOT change which resolvers are generated.

#### Scenario: xray TUN with DNS disabled
- **WHEN** the backend is xray, TUN is enabled, and DNS is disabled
- **THEN** the list SHALL contain `https://1.1.1.1/dns-query` and `https://8.8.8.8/dns-query` with path `proxy` and source `fallback`, and SHALL NOT contain the configured DNS servers

#### Scenario: xray with only domain-scoped servers
- **WHEN** the backend is xray, DNS is enabled, and every configured server is scoped to domains
- **THEN** the list SHALL contain the configured servers with their domain scope followed by `https://1.1.1.1/dns-query` with source `fallback` and scope all domains

#### Scenario: xray TUN bootstrap for a hostname node
- **WHEN** the backend is xray, TUN is enabled, and the node address is a hostname
- **THEN** the list SHALL start with UDP `1.1.1.1` and DoH `1.1.1.1` with path `direct`, source `bootstrap`, and scope that hostname

#### Scenario: sing-box TUN with DNS disabled
- **WHEN** the backend is sing-box, TUN is enabled, and DNS is disabled
- **THEN** the list SHALL contain DoH `1.1.1.1` with path `proxy` and source `fallback`

#### Scenario: sing-box server without detour
- **WHEN** the backend is sing-box, DNS is enabled, and a configured server has no detour
- **THEN** that entry SHALL have path `direct`

#### Scenario: No DNS section outside TUN
- **WHEN** the backend is xray or v2ray, TUN is off, DNS is disabled, and routing rules are enabled
- **THEN** the list SHALL contain one entry with path `system` and source `system`

#### Scenario: Imported profile supplies DNS
- **WHEN** the inputs come from a subscription node whose enabled imported profile carries DNS settings
- **THEN** entries built from those settings SHALL have source `profile`

#### Scenario: Wrong-family host overrides omitted from Effective DNS
- **WHEN** the backend is xray or v2ray, the query strategy is `Ipv4Only`, and a static host override maps `v6.example.com` only to `2001:db8::1`
- **THEN** `v6.example.com` SHALL NOT appear as a `static` entry in the Effective DNS list
