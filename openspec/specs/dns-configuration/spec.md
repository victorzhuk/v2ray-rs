## Purpose

Define DNS server protocols, named servers, query strategy, routing rules, FakeIP, cache, client subnet, host overrides, and backend-specific config generation.

## Requirements

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

### Requirement: DNS server detour per backend
The optional detour field on DNS servers SHALL be honored by the sing-box and xray config generators and ignored by the v2ray generator. Each backend SHALL express only the detour it can start against. sing-box SHALL emit a `detour` field naming the first proxy outbound for any detour other than "direct"; a detour of "direct" SHALL be expressed by omitting the field, because a DNS server carrying no detour is not dispatched through the proxy chain and sing-box refuses to start when a DNS server detours to an outbound that carries no settings. xray has no per-server detour field, so a detour of "direct" SHALL be expressed as a `tag` on the server object plus a routing rule sending that tag to the direct outbound; any other detour value SHALL be ignored, because it names the default route. A server with no detour SHALL keep the default behaviour of traversing the proxy.

#### Scenario: Detour emitted for sing-box
- **WHEN** a DNS server has a detour other than "direct" and the backend is sing-box
- **THEN** the generated server object SHALL include a "detour" field naming the tag of the first proxy outbound

#### Scenario: Direct detour omitted for sing-box
- **WHEN** a DNS server has a detour of "direct" and the backend is sing-box
- **THEN** the generated server object SHALL carry no "detour" field, and the generated configuration SHALL start against the backend rather than being rejected at service start

#### Scenario: Direct detour becomes a tag and a rule for xray
- **WHEN** a DNS server has a detour of "direct" and the backend is xray
- **THEN** the generated server object SHALL carry a tag identifying it as directly routed, and `routing.rules` SHALL contain a rule sending that `inboundTag` to the direct outbound ahead of the internal-resolver rule

#### Scenario: Detour ignored for v2ray/xray
- **WHEN** a DNS server has a detour the backend cannot express — any value for v2ray, or any value other than "direct" for xray
- **THEN** the generated DNS config SHALL NOT include any detour-related field or tag for that server, and the query SHALL follow the backend's default routing

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
The system SHALL support static domain-to-IP mappings that override DNS resolution. A mapping SHALL only be emitted for the address family the configured query strategy selects, because a host override that resolves to nothing usable is answered authoritatively as empty rather than falling through to the configured servers. A domain left with no address of the selected family SHALL be omitted from the generated mapping.

#### Scenario: Host override applied in v2ray/xray
- **WHEN** the user adds a host override "ads.example.com" → "127.0.0.1" and the backend is v2ray or xray
- **THEN** the generated config SHALL include this mapping in the dns.hosts object

#### Scenario: Host override applied in sing-box
- **WHEN** the user adds a host override "ads.example.com" → "127.0.0.1" and the backend is sing-box
- **THEN** the generated config SHALL include a hosts-type DNS server with the static mapping

#### Scenario: No host overrides by default
- **WHEN** no host overrides are configured
- **THEN** the hosts section SHALL be omitted from the generated config

#### Scenario: Override addresses of the wrong family are dropped
- **WHEN** a static host override maps a domain only to addresses the query strategy will not use
- **THEN** that domain SHALL be absent from the generated mapping

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

### Requirement: DNS validation covers FakeIP CIDR ranges
The DNS model SHALL validate FakeIP IPv4 and IPv6 CIDR ranges when FakeIP is enabled. Duplicate server tags, invalid rule targets, and invalid client subnet values were already validated before this requirement was formally specified.

#### Scenario: Invalid FakeIP IPv4 range
- **WHEN** FakeIP is enabled and the user configures an invalid FakeIP IPv4 CIDR range
- **THEN** the DNS model rejects the configuration with a validation error

#### Scenario: Invalid FakeIP IPv6 range
- **WHEN** FakeIP is enabled and the user configures an invalid FakeIP IPv6 CIDR range
- **THEN** the DNS model rejects the configuration with a validation error

#### Scenario: FakeIP validation skipped when disabled
- **WHEN** FakeIP is disabled
- **THEN** the DNS model SHALL NOT validate the CIDR range values, allowing them to hold any string without error until FakeIP is enabled

### Requirement: DNS validation rejects an empty server list

The DNS model SHALL reject a configuration where DNS is enabled but no servers are configured, since FakeIP alone cannot answer real queries.

#### Scenario: DNS enabled with no servers
- **WHEN** DNS is enabled and `servers` is empty
- **THEN** the DNS model rejects the configuration with a validation error

### Requirement: Backend DNS protocol compatibility matrix
The system SHALL define per-backend DNS protocol support normatively: sing-box supports UDP, TCP, DoH, DoT, DoQ, and H3 natively; xray supports all except H3; v2ray supports only UDP, TCP, and DoH. Protocols a backend does not support SHALL be downgraded to DoH at config-generation time. The compatibility and downgrade mapping SHALL live in one core function that both the config generators and the UI consult.

#### Scenario: Downgrade mapping is single-sourced
- **WHEN** the UI or a config generator needs to know a protocol's effective form on a backend
- **THEN** both SHALL consult the same core compatibility function, so UI messaging and generated configs cannot drift

#### Scenario: sing-box passes protocols through natively
- **WHEN** a DNS server is configured with any supported protocol and the backend is sing-box
- **THEN** the generated config SHALL use the configured protocol without downgrade

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

### Requirement: Private DNS servers routed through the proxy are flagged
The system SHALL identify a DNS server whose address is an IP literal in a loopback (`127.0.0.0/8`, `::1`), private (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`), link-local (`169.254.0.0/16`, `fe80::/10`), or unique-local (`fc00::/7`) range and whose queries the generated config sends through the proxy: on sing-box, a server with a detour other than "direct"; on xray, any server not expressed as directly routed (a "direct" detour under TUN). Such a server reaches the proxy server's network, not the user's. The system SHALL NOT reject such a server, because a resolver on the proxy server's own network is a valid setup; config generation SHALL log one warning per flagged server naming its tag and address. Hostname-addressed servers and the v2ray backend SHALL NOT be flagged.

#### Scenario: Loopback server with proxy detour on sing-box
- **WHEN** the backend is sing-box and a server `domestic` is `udp 127.0.0.1` with detour `proxy`
- **THEN** the server SHALL be flagged, config generation SHALL succeed, and a warning naming `domestic` and `127.0.0.1` SHALL be logged

#### Scenario: Loopback server without direct routing on xray
- **WHEN** the backend is xray with TUN enabled and a server is `udp 127.0.0.1` with detour `proxy` or no detour
- **THEN** the server SHALL be flagged

#### Scenario: Direct private server is not flagged
- **WHEN** a server is `udp 192.168.1.1` with detour `direct` and the backend is sing-box, or xray with TUN enabled
- **THEN** the server SHALL NOT be flagged

#### Scenario: Public server is not flagged
- **WHEN** a server is `udp 1.1.1.1` with detour `proxy`
- **THEN** the server SHALL NOT be flagged

### Requirement: Auto-derived DNS rules require their server tags
When DNS is enabled and `use_custom_rules` is false, the generators SHALL emit derived DNS rules for proxy-action domains only when a server tagged `remote` exists, and for direct-action domains only when a server tagged `domestic` exists. For each missing tag that derived rules would have used, generation SHALL log a warning naming the tag and the number of skipped domain entries, and SHALL NOT emit any DNS rule or `domains` entry referencing a server that is not in the generated config.

#### Scenario: Missing domestic tag on sing-box
- **WHEN** the backend is sing-box, auto-derived rules are active, servers are tagged `remote` and `lan`, and a direct routing rule has GeoSite `category-ru`
- **THEN** `dns.rules` SHALL contain no rule with `"server": "domestic"`, a warning naming `domestic` SHALL be logged, and the config SHALL pass `sing-box check`

#### Scenario: Missing remote tag on xray
- **WHEN** the backend is xray, auto-derived rules are active, no server is tagged `remote`, and a proxy routing rule has domain pattern `example.com`
- **THEN** no DNS server SHALL carry `domain:example.com` in `domains` and a warning naming `remote` SHALL be logged

#### Scenario: Standard tags unaffected
- **WHEN** servers are tagged `remote` and `domestic`
- **THEN** derived DNS rules SHALL be emitted as before and no warning SHALL be logged

### Requirement: Effective DNS resolver set is derivable
The system SHALL derive, for a backend, the DNS settings, the TUN settings, and the routing rules used to generate a config, the ordered list of resolvers the generated config uses. Each entry SHALL state the resolver address, its transport, its path (`direct`, `proxy`, `routing` when the backend's routing rules decide, `system` for the operating-system resolver, or `static` for host overrides), its source (`user`, `fallback`, `bootstrap`, `system`, or `profile` when a subscription's imported profile supplied the DNS settings), and its scope (all domains or the listed domains). The list SHALL match the resolvers present in the generated config for the same inputs. Deriving it SHALL NOT change which resolvers are generated.

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
