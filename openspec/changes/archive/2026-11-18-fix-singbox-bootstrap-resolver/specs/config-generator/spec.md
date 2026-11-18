# Spec Delta

## MODIFIED Requirements

### Requirement: Generate sing-box configuration
The system SHALL generate a valid JSON configuration file in sing-box's configuration schema. When DNS is enabled, the DNS section SHALL include typed server objects, DNS rules, strategy, FakeIP, cache settings, and client subnet, and the route section SHALL include `default_domain_resolver` set to the tag of the first DNS server whose address is a literal IP and whose detour is absent or `"direct"`, falling back to a generated bootstrap DNS server (`sys-dns-bootstrap`) emitted in `dns.servers`. When TUN is enabled, the bootstrap SHALL be a direct-dialed DoH server at an IP-literal endpoint, `{"tag": "sys-dns-bootstrap", "type": "https", "server": "1.1.1.1", "server_port": 443, "path": "/dns-query"}` with no `detour`, and no `local` server SHALL be emitted. When TUN is disabled, the bootstrap SHALL be `{"tag": "sys-dns-bootstrap", "type": "local"}`. Inbound `listen` SHALL be taken from `AppSettings::listen_address` (default `127.0.0.1`), and the `mixed` inbound SHALL NOT emit `udp_disabled: true` so UDP remains enabled.

#### Scenario: sing-box basic config
- **WHEN** the user has one enabled Shadowsocks node with sing-box selected
- **THEN** the system SHALL generate a sing-box JSON config with mixed inbound on 127.0.0.1, Shadowsocks outbound, direct outbound, and route rules

#### Scenario: Custom listen address propagated to both sing-box inbounds
- **WHEN** the user sets `listen_address` to `192.168.1.10`
- **THEN** both the `mixed` and `http` inbounds in the generated sing-box config SHALL have `"listen": "192.168.1.10"` while ports remain unchanged

#### Scenario: sing-box mixed inbound supports UDP
- **WHEN** the system generates a sing-box config
- **THEN** the SOCKS-capable inbound SHALL have `"type": "mixed"` and SHALL NOT contain `"udp_disabled": true`

#### Scenario: sing-box DNS with typed servers
- **WHEN** DNS is enabled with a DoH server (tag "remote") and UDP server (tag "domestic")
- **THEN** the sing-box config SHALL include dns.servers with typed objects: {"type": "https", "tag": "remote", ...} and {"type": "udp", "tag": "domestic", ...}

#### Scenario: sing-box DNS with FakeIP enabled
- **WHEN** DNS is enabled and FakeIP is enabled with ranges 198.18.0.0/15 and fc00::/18
- **THEN** the sing-box config SHALL include a `dns.servers` entry `{"type": "fakeip", "tag": "fakeip", "inet4_range": "198.18.0.0/15", "inet6_range": "fc00::/18"}`, a `dns.rules` entry `{"query_type": ["A", "AAAA"], "server": "fakeip"}`, and `dns.final` SHALL point to a real DNS server, never to "fakeip"

#### Scenario: sing-box DNS with custom rules
- **WHEN** DNS is enabled with custom DNS rules (GeoSite "google" → "remote", domain suffix "cn" → "domestic")
- **THEN** the sing-box config dns.rules SHALL contain rule objects with rule_set/domain_suffix fields routing matching queries to the specified server tags

#### Scenario: sing-box DNS with host overrides
- **WHEN** DNS is enabled with host overrides {"ads.example.com": "127.0.0.1"}
- **THEN** the sing-box config SHALL include a hosts-type DNS server with a `predefined` static mapping, and `dns.rules` SHALL start with a rule routing the overridden domains to the "hosts" server

#### Scenario: sing-box default_domain_resolver is set when DNS is enabled
- **WHEN** DNS is enabled with 2 or more servers
- **THEN** the sing-box config route section SHALL include `default_domain_resolver` set to a non-proxy-detoured DNS server tag

#### Scenario: sing-box default_domain_resolver skips proxy-detoured IP servers
- **WHEN** DNS is enabled without TUN using `DnsConfig::default()` or the Cloudflare/Google preset (`"remote"` at `1.1.1.1` or `8.8.8.8` with `detour = "proxy"`, `"domestic"` at an IP literal with `detour = "direct"`) and the proxy outbound is addressed by hostname
- **THEN** `route.default_domain_resolver` SHALL be `"domestic"` rather than `"remote"`, so dialing the proxy hostname never recurses through `"remote"` -> `"proxy-0"`

#### Scenario: sing-box emits direct DoH bootstrap under TUN when no direct IP server exists
- **WHEN** DNS is enabled, TUN is enabled, and no configured DNS server is both IP-literal and non-proxy-detoured
- **THEN** `dns.servers` SHALL include `{"tag": "sys-dns-bootstrap", "type": "https", "server": "1.1.1.1", "server_port": 443, "path": "/dns-query"}` without `detour`, both `route.default_domain_resolver` and any hostname-addressed DNS server's `domain_resolver` SHALL be `"sys-dns-bootstrap"`, and no `local` server SHALL be emitted

#### Scenario: sing-box emits local bootstrap without TUN when no direct IP server exists
- **WHEN** DNS is enabled, TUN is disabled, and no configured DNS server is both IP-literal and non-proxy-detoured
- **THEN** `dns.servers` SHALL include `{"tag": "sys-dns-bootstrap", "type": "local"}` without a proxy detour, and both `route.default_domain_resolver` and any hostname-addressed DNS server's `domain_resolver` SHALL be `"sys-dns-bootstrap"`

#### Scenario: Effective DNS reports the bootstrap resolver
- **WHEN** the bootstrap resolver is selected
- **THEN** the Effective DNS summary SHALL list it with address `https://1.1.1.1/dns-query`, transport DoH, path `direct`, source `bootstrap`, scope `all` under TUN, and address `local`, no transport, path `system`, source `system`, scope `all` without TUN

#### Scenario: sing-box DNS with detour
- **WHEN** DNS is enabled and the "remote" server has a detour set to anything other than "direct"
- **THEN** the sing-box config dns.servers entry for "remote" SHALL include "detour" set to the tag of the first proxy outbound

#### Scenario: sing-box DNS with strategy and client subnet
- **WHEN** DNS is enabled with strategy Ipv6Only and client_subnet "2001:db8::1"
- **THEN** the sing-box config dns section SHALL include "strategy": "ipv6_only" and every emitted configured upstream DNS server object SHALL carry "client_subnet": "2001:db8::1" (the generated bootstrap resolver carries none)
