## MODIFIED Requirements

### Requirement: Generate v2ray-compatible configuration
The system SHALL generate a valid JSON configuration file for v2ray/xray containing inbound, outbound, routing, and DNS sections. When DNS is enabled, the DNS section SHALL reflect the full DNS configuration model including multiple servers, query strategy, hosts, cache settings, and client IP.

#### Scenario: Basic SOCKS5 + HTTP inbound with single proxy outbound
- **WHEN** the user has one enabled VLESS node and default settings (SOCKS5 port 1080, HTTP port 1081)
- **THEN** the system SHALL generate a JSON config with SOCKS5 inbound on 1080, HTTP inbound on 1081, a VLESS outbound, and a "freedom" direct outbound

#### Scenario: Multiple proxy nodes with auto-resolve
- **WHEN** the user has multiple enabled nodes and an auto-resolve strategy selected
- **THEN** the system SHALL generate a config for the active connection candidate and refresh it for each candidate attempt

#### Scenario: DNS with multiple servers and query strategy
- **WHEN** DNS is enabled with 3 servers (remote DoH, domestic UDP, adblock DoT) and strategy Ipv4Only
- **THEN** the v2ray config SHALL include a "dns" section with all 3 servers mapped to v2ray address format, queryStrategy "UseIPv4", and per-server domains from DNS rules

#### Scenario: DNS with hosts overrides
- **WHEN** DNS is enabled with host overrides {"ads.example.com": "127.0.0.1"}
- **THEN** the v2ray config DNS section SHALL include a "hosts" object with the mapping

#### Scenario: DNS with cache disabled and client IP
- **WHEN** DNS is enabled with disable_cache=true and client_subnet="203.0.113.1"
- **THEN** the v2ray config DNS section SHALL include "disableCache": true and "clientIp": "203.0.113.1"

#### Scenario: DNS protocol fallback for v2ray (DoT/DoQ/H3)
- **WHEN** a DNS server uses DoT, DoQ, or H3 protocol and backend is v2ray
- **THEN** the system SHALL fall back to DoH format for that server and log a warning

#### Scenario: DNS protocol fallback for xray (H3)
- **WHEN** a DNS server uses H3 protocol and backend is xray
- **THEN** the system SHALL fall back to DoH format for that server and log a warning

#### Scenario: Detour ignored for v2ray/xray
- **WHEN** a DNS server has a detour configured and backend is v2ray or xray
- **THEN** the generated DNS config SHALL NOT include any detour field

### Requirement: Generate sing-box configuration
The system SHALL generate a valid JSON configuration file in sing-box's configuration schema. When DNS is enabled, the DNS section SHALL include typed server objects, DNS rules, strategy, FakeIP, cache settings, and client subnet.

#### Scenario: sing-box basic config
- **WHEN** the user has one enabled Shadowsocks node with sing-box selected
- **THEN** the system SHALL generate a sing-box JSON config with mixed inbound, Shadowsocks outbound, direct outbound, and route rules

#### Scenario: sing-box DNS with typed servers
- **WHEN** DNS is enabled with a DoH server (tag "remote") and UDP server (tag "domestic")
- **THEN** the sing-box config SHALL include dns.servers with typed objects: {"type": "https", "tag": "remote", ...} and {"type": "udp", "tag": "domestic", ...}

#### Scenario: sing-box DNS with FakeIP enabled
- **WHEN** DNS is enabled and FakeIP is enabled with ranges 198.18.0.0/15 and fc00::/18
- **THEN** the sing-box config SHALL include a fakeip server in dns.servers and fakeip configuration with the specified ranges

#### Scenario: sing-box DNS with custom rules
- **WHEN** DNS is enabled with custom DNS rules (GeoSite "google" → "remote", domain suffix "cn" → "domestic")
- **THEN** the sing-box config dns.rules SHALL contain rule objects with rule_set/domain_suffix fields routing matching queries to the specified server tags

#### Scenario: sing-box DNS with host overrides
- **WHEN** DNS is enabled with host overrides {"ads.example.com": "127.0.0.1"}
- **THEN** the sing-box config SHALL include a hosts-type DNS server with the static mapping

#### Scenario: sing-box DNS with detour
- **WHEN** DNS is enabled and the "remote" server has detour "proxy-0"
- **THEN** the sing-box config dns.servers entry for "remote" SHALL include "detour": "proxy-0"

#### Scenario: sing-box DNS with strategy and client subnet
- **WHEN** DNS is enabled with strategy Ipv6Only and client_subnet "2001:db8::1"
- **THEN** the sing-box config dns section SHALL include "strategy": "ipv6_only" and "client_subnet": "2001:db8::1"

### Requirement: Embed routing rules in config
The system SHALL translate the user's routing rules into the backend-specific routing section of the generated config.

#### Scenario: GeoIP direct rule in v2ray config
- **WHEN** the user has a rule "GeoIP:RU → direct"
- **THEN** the v2ray config routing section SHALL contain a rule matching geoip "ru" pointing to the direct outbound tag

#### Scenario: GeoSite proxy rule in sing-box config
- **WHEN** the user has a rule "GeoSite:google → proxy"
- **THEN** the sing-box config route section SHALL contain a rule matching geosite "google" pointing to the proxy outbound tag

### Requirement: Atomic config file writes
The system SHALL write generated config files atomically (write to temp file, then rename) to prevent corruption.

#### Scenario: Crash during write
- **WHEN** the app crashes during config generation
- **THEN** the previously valid config file SHALL remain intact

### Requirement: Reactive config regeneration
The system SHALL automatically regenerate the config file when subscription data, routing rules, or DNS settings change.

#### Scenario: Subscription update triggers regen
- **WHEN** a subscription is updated with new nodes
- **THEN** the system SHALL regenerate the config within 1 second

#### Scenario: Routing rule change triggers regen
- **WHEN** the user adds or modifies a routing rule
- **THEN** the system SHALL regenerate the config immediately

#### Scenario: DNS settings change triggers regen
- **WHEN** the user modifies any DNS setting (servers, rules, strategy, FakeIP, etc.)
- **THEN** the system SHALL regenerate the config immediately
