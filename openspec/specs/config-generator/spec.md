## MODIFIED Requirements

### Requirement: Generate v2ray-compatible configuration
The system SHALL generate a valid JSON configuration file for v2ray/xray containing inbound, outbound, routing, and DNS sections. When DNS is enabled, the DNS section SHALL reflect the full DNS configuration model including multiple servers, query strategy, hosts, cache settings, and client IP. Inbound `listen` SHALL be taken from `AppSettings::listen_address` (default `127.0.0.1`), and the SOCKS-capable inbound SHALL declare `settings.udp = true`.

#### Scenario: Basic SOCKS5 + HTTP inbound with single proxy outbound
- **WHEN** the user has one enabled VLESS node and default settings (SOCKS5 port 1080, HTTP port 1081, listen address 127.0.0.1)
- **THEN** the system SHALL generate a JSON config with SOCKS5 inbound on 127.0.0.1:1080, HTTP inbound on 127.0.0.1:1081, a VLESS outbound, and a "freedom" direct outbound

#### Scenario: Custom listen address propagated to both inbounds
- **WHEN** the user sets `listen_address` to `0.0.0.0`
- **THEN** both the SOCKS and HTTP inbounds in the generated v2ray/xray config SHALL have `"listen": "0.0.0.0"` while ports remain unchanged

#### Scenario: SOCKS inbound has UDP enabled
- **WHEN** the system generates a v2ray or xray config
- **THEN** the SOCKS inbound SHALL contain `"settings": { "udp": true }`

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
The system SHALL generate a valid JSON configuration file in sing-box's configuration schema. When DNS is enabled, the DNS section SHALL include typed server objects, DNS rules, strategy, FakeIP, cache settings, and client subnet. Inbound `listen` SHALL be taken from `AppSettings::listen_address` (default `127.0.0.1`), and the `mixed` inbound SHALL NOT emit `udp_disabled: true` so UDP remains enabled.

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

### Requirement: Defensive listen-address validation in writer
The config writer SHALL validate `AppSettings::listen_address` before invoking any generator. If the value is not a parseable IPv4 or IPv6 literal, the writer SHALL substitute `127.0.0.1`, log a warning, and proceed; it SHALL NOT abort writing.

#### Scenario: Invalid listen address falls back to loopback
- **WHEN** `AppSettings::listen_address` is `"not-an-ip"` and the user triggers a config regeneration
- **THEN** the generated config SHALL contain `"listen": "127.0.0.1"` on every inbound and the writer SHALL log a warning identifying the invalid value

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
The system SHALL automatically regenerate the config file when subscription data, manual nodes, routing rules, or DNS settings change, with behavior depending on connection state.
- When the backend is stopped, changes SHALL regenerate the config immediately.
- When the backend is starting or running, changes SHALL be persisted but SHALL NOT replace the active runtime config until the user applies restart or reconnects.

#### Scenario: Subscription update triggers regen
- **WHEN** a subscription is updated with new nodes
- **THEN** the system SHALL regenerate the config within 1 second

#### Scenario: Disconnected routing change triggers regen
- **WHEN** the backend is stopped and the user changes a routing rule
- **THEN** the system regenerates the config immediately

#### Scenario: Connected DNS change waits for restart
- **WHEN** the backend is connected and the user changes DNS settings
- **THEN** the new settings are persisted, the active runtime config is marked as restart-required, and the running backend continues using the previous launched config

#### Scenario: Disconnected manual node change triggers regen
- **WHEN** the backend is stopped and the user adds, edits, deletes, or toggles the enabled state of a manual node
- **THEN** the system SHALL regenerate the config immediately

#### Scenario: Connected manual node change waits for restart
- **WHEN** the backend is connected and the user adds, edits, deletes, or toggles the enabled state of a manual node
- **THEN** the change is persisted, but the active runtime config is not replaced until the user applies restart or reconnects later

### Requirement: Generated configs live in runtime directory
The system SHALL write generated backend config files to the active profile's `runtime_dir/generated/` by default. The existing `backend.config_output_dir` user setting SHALL continue to override the output directory when set.

#### Scenario: Default output path
- **WHEN** the user has not set `backend.config_output_dir` and the active profile is `Production`
- **THEN** the generated `xray.json`/`v2ray.json`/`sing-box.json` SHALL be written under `runtime_dir/generated/`

#### Scenario: User override still wins
- **WHEN** the user has set `backend.config_output_dir` to `/etc/v2ray-rs/configs`
- **THEN** the generated config SHALL be written under `/etc/v2ray-rs/configs/`

#### Scenario: Generated configs are profile-isolated
- **WHEN** the same user runs the binary with `--profile production` and `--profile development` at different times
- **THEN** each profile SHALL maintain its own generated config files in its own `runtime_dir/generated/`

### Requirement: Generate sing-box TUN inbound
When TUN is enabled and the backend is sing-box, the system SHALL add a native `tun` inbound to the generated config alongside the existing `mixed`/`http` inbounds. The inbound SHALL set `auto_route: true`, the configured interface name, address(es), and MTU, and SHALL map the advanced settings: `stack`, `strict_route`, `dns_hijack` → `dns_mode`, and `exclude_routes` → `route_exclude_address`. The route section SHALL set `auto_detect_interface: true`.

#### Scenario: sing-box TUN inbound emitted when enabled
- **WHEN** TUN is enabled with sing-box, address `172.19.0.1/30`, MTU 1500, stack system, and strict route on
- **THEN** the generated config inbounds SHALL include a `{ "type": "tun", "auto_route": true, "address": ["172.19.0.1/30"], "mtu": 1500, "stack": "system", "strict_route": true }` entry, and the route section SHALL include `"auto_detect_interface": true`

#### Scenario: Excluded routes mapped
- **WHEN** TUN is enabled with `exclude_routes` `["192.168.0.0/16"]`
- **THEN** the sing-box tun inbound SHALL include `"route_exclude_address": ["192.168.0.0/16"]`

#### Scenario: No sing-box TUN inbound when disabled
- **WHEN** TUN is disabled
- **THEN** the generated sing-box config SHALL NOT contain any inbound of type `tun`

### Requirement: Generate xray TUN inbound
When TUN is enabled and the backend is xray, the system SHALL add a native `tun` protocol inbound to the generated config alongside the existing socks/http inbounds, with the configured name, MTU, gateway address(es), DNS, `autoOutboundsInterface: "auto"`, and sniffing enabled.

#### Scenario: xray TUN inbound emitted when enabled
- **WHEN** TUN is enabled with xray, address `198.18.0.1/30`, and MTU 1500
- **THEN** the generated config inbounds SHALL include a `{ "protocol": "tun", "settings": { "name": "...", "mtu": 1500, "gateway": ["198.18.0.1/30"], "autoOutboundsInterface": "auto" } }` entry with sniffing enabled

#### Scenario: No xray TUN inbound when disabled
- **WHEN** TUN is disabled
- **THEN** the generated xray config SHALL NOT contain any `tun`-protocol inbound

### Requirement: v2ray backend never emits a TUN inbound
When the backend is v2ray, the system SHALL NOT emit a TUN inbound regardless of the persisted `tun.enabled` flag, because v2ray-core has no native TUN support.

#### Scenario: v2ray ignores TUN
- **WHEN** the backend is v2ray and `tun.enabled` is true
- **THEN** the generated v2ray config SHALL contain only the socks and http inbounds and no tun inbound
