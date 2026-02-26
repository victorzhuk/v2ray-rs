## ADDED Requirements

### Requirement: DNS protocol types
The system SHALL support the following DNS protocol types: UDP (plain), TCP, DoH (DNS-over-HTTPS), DoT (DNS-over-TLS), DoQ (DNS-over-QUIC), and H3 (DNS-over-HTTP/3).

#### Scenario: UDP address formatting
- **WHEN** a DNS server is configured with protocol UDP and address "8.8.8.8"
- **THEN** the system SHALL represent it as "8.8.8.8:53" (default port always appended)

#### Scenario: UDP with custom port
- **WHEN** a DNS server is configured with protocol UDP, address "8.8.8.8", and port 5353
- **THEN** the system SHALL represent it as "8.8.8.8:5353"

#### Scenario: TCP address formatting
- **WHEN** a DNS server is configured with protocol TCP and address "8.8.8.8"
- **THEN** the system SHALL produce the address "tcp://8.8.8.8:53"

#### Scenario: DoH address formatting
- **WHEN** a DNS server is configured with protocol DoH and address "1.1.1.1"
- **THEN** the system SHALL produce the address "https://1.1.1.1/dns-query"

#### Scenario: DoT address formatting
- **WHEN** a DNS server is configured with protocol DoT and address "dns.google"
- **THEN** the system SHALL produce the address "tls://dns.google"

#### Scenario: DoQ address formatting
- **WHEN** a DNS server is configured with protocol DoQ and address "dns.adguard.com"
- **THEN** the system SHALL produce the address "quic://dns.adguard.com"

#### Scenario: H3 address formatting
- **WHEN** a DNS server is configured with protocol H3 and address "dns.google"
- **THEN** the system SHALL produce the address "h3://dns.google/dns-query"

### Requirement: Named DNS servers
The system SHALL support multiple named DNS servers. Each server SHALL have a tag (unique string identifier), protocol, address, optional port, and optional detour (outbound tag for routing DNS traffic).

#### Scenario: Default DNS servers
- **WHEN** no DNS configuration exists (fresh install)
- **THEN** the system SHALL provide two default servers: a "remote" server (DoH, 1.1.1.1) and a "domestic" server (UDP, 223.5.5.5)

#### Scenario: Add custom server
- **WHEN** the user adds a DNS server with tag "adguard", protocol DoH, address "dns.adguard.com"
- **THEN** the server SHALL be persisted and available for DNS rule assignment

#### Scenario: Duplicate tag rejected
- **WHEN** the user attempts to add a server with a tag that already exists
- **THEN** the system SHALL reject the addition with a validation error

### Requirement: DNS query strategy
The system SHALL support an IP query strategy setting with values: PreferIpv4, PreferIpv6, Ipv4Only, Ipv6Only.

#### Scenario: Default strategy
- **WHEN** no strategy is explicitly set
- **THEN** the system SHALL default to PreferIpv4

#### Scenario: Strategy applied globally
- **WHEN** the user sets strategy to Ipv4Only
- **THEN** all DNS config generation SHALL reflect IPv4-only query mode for the selected backend

### Requirement: DNS routing rules
The system SHALL support user-defined DNS routing rules. Each rule maps a match condition (GeoSite category or domain suffix) to a DNS server tag. A `use_custom_rules` toggle controls the mode:
- When `use_custom_rules` is false (default), the system SHALL auto-derive DNS routing from the existing routing rules
- When `use_custom_rules` is true, the system SHALL use only the user-defined DNS rules

#### Scenario: Auto-derived DNS rules (default)
- **WHEN** DNS is enabled and `use_custom_rules` is false
- **THEN** the system SHALL derive DNS server assignments from routing rules: proxy-action domains use the "remote" server, direct-action domains use the "domestic" server

#### Scenario: Custom rules mode active
- **WHEN** `use_custom_rules` is true
- **THEN** the system SHALL use only user-defined DNS rules and SHALL NOT auto-derive from routing rules

#### Scenario: Saved rules ignored in auto-derive mode
- **WHEN** `use_custom_rules` is false but the user has previously saved custom DNS rules
- **THEN** the saved rules SHALL be preserved in settings but NOT used for config generation

#### Scenario: DNS rule with GeoSite match
- **WHEN** a DNS rule matches GeoSite "google" to server tag "remote"
- **THEN** DNS queries for domains in the google geosite category SHALL be routed to the "remote" DNS server

#### Scenario: DNS rule with domain suffix match
- **WHEN** a DNS rule matches domain suffix "example.com" to server tag "domestic"
- **THEN** DNS queries for example.com and all its subdomains SHALL be routed to the "domestic" DNS server

### Requirement: Detour is sing-box only
The optional detour field on DNS servers SHALL only be used by the sing-box config generator. V2ray and xray generators SHALL ignore the detour field.

#### Scenario: Detour emitted for sing-box
- **WHEN** a DNS server has detour "proxy-0" and the backend is sing-box
- **THEN** the generated server object SHALL include "detour": "proxy-0"

#### Scenario: Detour ignored for v2ray/xray
- **WHEN** a DNS server has detour "proxy-0" and the backend is v2ray or xray
- **THEN** the generated DNS config SHALL NOT include any detour-related fields for that server

### Requirement: FakeIP configuration
The system SHALL support FakeIP configuration for the sing-box backend. FakeIP SHALL have an enable toggle, IPv4 range, and IPv6 range.

#### Scenario: FakeIP defaults
- **WHEN** FakeIP is not explicitly configured
- **THEN** FakeIP SHALL be disabled with default ranges 198.18.0.0/15 (IPv4) and fc00::/18 (IPv6)

#### Scenario: FakeIP ignored for v2ray/xray
- **WHEN** FakeIP is enabled but the selected backend is v2ray or xray
- **THEN** the config generator SHALL skip the FakeIP configuration entirely

### Requirement: DNS cache control
The system SHALL support a toggle to disable DNS caching.

#### Scenario: Cache enabled by default
- **WHEN** no cache setting is explicitly configured
- **THEN** DNS caching SHALL be enabled (disable_cache = false)

#### Scenario: Cache disabled
- **WHEN** the user disables DNS cache
- **THEN** the generated config SHALL include the disable_cache flag for the selected backend

### Requirement: EDNS client subnet
The system SHALL support an optional EDNS client subnet IP address for geo-aware DNS responses.

#### Scenario: No client subnet by default
- **WHEN** no client subnet is configured
- **THEN** the generated DNS config SHALL omit the client subnet / clientIp field

#### Scenario: Client subnet set
- **WHEN** the user sets client subnet to "203.0.113.1"
- **THEN** the generated config SHALL include the client subnet IP in the appropriate backend field (clientIp for v2ray, client_subnet for sing-box)

### Requirement: Static host overrides
The system SHALL support static domain-to-IP mappings that override DNS resolution.

#### Scenario: Host override applied in v2ray/xray
- **WHEN** the user adds a host override "ads.example.com" → "127.0.0.1" and the backend is v2ray or xray
- **THEN** the generated config SHALL include this mapping in the dns.hosts object

#### Scenario: Host override applied in sing-box
- **WHEN** the user adds a host override "ads.example.com" → "127.0.0.1" and the backend is sing-box
- **THEN** the generated config SHALL include a hosts-type DNS server with the static mapping

#### Scenario: No host overrides by default
- **WHEN** no host overrides are configured
- **THEN** the hosts section SHALL be omitted from the generated config

### Requirement: Backward-compatible deserialization with migration
The system SHALL deserialize existing settings.toml files (with the old minimal DnsConfig format) without error, migrating old field values to the new model.

#### Scenario: Old config migrates server addresses
- **WHEN** a settings.toml contains `[dns]` with `enabled = true`, `remote = { protocol = "doh", address = "8.8.8.8" }`, and `domestic = { protocol = "plain", address = "114.114.114.114" }`
- **THEN** the system SHALL load successfully with `enabled = true`, `servers` containing a "remote" server (DoH, 8.8.8.8) and a "domestic" server (UDP, 114.114.114.114), and all new fields at defaults

#### Scenario: Old config does not lose user DNS settings
- **WHEN** a settings.toml has old-format `remote` and `domestic` fields with non-default addresses
- **THEN** those addresses SHALL be preserved in the migrated `servers` list, NOT replaced by defaults

#### Scenario: Fresh config with no dns section
- **WHEN** a settings.toml has no `[dns]` section at all
- **THEN** the system SHALL use `DnsConfig::default()` with DNS disabled and two default servers

#### Scenario: New format loads directly
- **WHEN** a settings.toml contains `[dns]` with `servers` array in the new format
- **THEN** the system SHALL load the new format directly without migration
