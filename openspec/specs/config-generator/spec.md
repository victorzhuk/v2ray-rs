## Purpose

Defines how the system generates backend-specific JSON configurations for sing-box, xray, and v2ray from proxy nodes, routing rules, DNS settings, and TUN preferences.

## Requirements

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
The system SHALL generate a valid JSON configuration file in sing-box's configuration schema. When DNS is enabled, the DNS section SHALL include typed server objects, DNS rules, strategy, FakeIP, cache settings, and client subnet, and the route section SHALL include `default_domain_resolver` set to the tag of the first DNS server whose address is a literal IP, falling back to the first server's tag. Inbound `listen` SHALL be taken from `AppSettings::listen_address` (default `127.0.0.1`), and the `mixed` inbound SHALL NOT emit `udp_disabled: true` so UDP remains enabled.

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
- **THEN** the sing-box config route section SHALL include `default_domain_resolver` set to a DNS server tag

#### Scenario: sing-box DNS with detour
- **WHEN** DNS is enabled and the "remote" server has a detour set to anything other than "direct"
- **THEN** the sing-box config dns.servers entry for "remote" SHALL include "detour" set to the tag of the first proxy outbound

#### Scenario: sing-box DNS with strategy and client subnet
- **WHEN** DNS is enabled with strategy Ipv6Only and client_subnet "2001:db8::1"
- **THEN** the sing-box config dns section SHALL include "strategy": "ipv6_only" and "client_subnet": "2001:db8::1"

### Requirement: Defensive listen-address validation in writer
The config writer SHALL validate `AppSettings::listen_address` before invoking any generator. If the value is not a parseable IPv4 or IPv6 literal, the writer SHALL substitute `127.0.0.1`, log a warning, and proceed; it SHALL NOT abort writing.

#### Scenario: Invalid listen address falls back to loopback
- **WHEN** `AppSettings::listen_address` is `"not-an-ip"` and the user triggers a config regeneration
- **THEN** the generated config SHALL contain `"listen": "127.0.0.1"` on every inbound and the writer SHALL log a warning identifying the invalid value

### Requirement: Embed routing rules in config
The system SHALL translate the user's routing rules into the backend-specific routing section of the generated config. For sing-box, GeoIP/GeoSite rules SHALL reference `type: remote` rule-sets without a `download_detour` field, so rule-set downloads go through sing-box's default outbound (the proxy), and any config referencing at least one remote rule-set SHALL enable `experimental.cache_file` with an absolute path under the profile's cache directory so fetched rule-sets persist across restarts.

#### Scenario: GeoIP direct rule in v2ray config
- **WHEN** the user has a rule "GeoIP:RU → direct"
- **THEN** the v2ray config routing section SHALL contain a rule matching geoip "ru" pointing to the direct outbound tag

#### Scenario: GeoSite proxy rule in sing-box config
- **WHEN** the user has a rule "GeoSite:google → proxy"
- **THEN** the sing-box config route section SHALL contain a rule matching geosite "google" pointing to the proxy outbound tag

#### Scenario: sing-box remote rule-sets download via the default outbound
- **WHEN** the user has any GeoIP or GeoSite rule and sing-box is the selected backend
- **THEN** each emitted `route.rule_set` entry SHALL have `type: "remote"` and SHALL NOT contain a `download_detour` field

#### Scenario: sing-box rule-set cache enabled
- **WHEN** the generated sing-box config references at least one remote rule-set
- **THEN** the config SHALL contain `experimental.cache_file` with `"enabled": true` and an absolute `path` under the profile's cache directory

#### Scenario: FakeIP mappings persisted when cache file present
- **WHEN** the generated sing-box config enables both the cache file and FakeIP
- **THEN** `experimental.cache_file` SHALL include `"store_fakeip": true`

#### Scenario: No cache file without remote rule-sets
- **WHEN** the routing rules reference no GeoIP or GeoSite rule-sets and FakeIP is disabled
- **THEN** the sing-box config SHALL NOT emit an `experimental.cache_file` section

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
When TUN is enabled and the backend is sing-box, the system SHALL add a native `tun` inbound to the generated config alongside the existing `mixed`/`http` inbounds. The inbound SHALL set `auto_route: true`, the configured interface name, address(es), MTU, `stack`, `strict_route`, and `exclude_routes` → `route_exclude_address`. The legacy `sniff`/`dns_mode` inbound fields SHALL NOT be emitted (removed in sing-box 1.13.0); instead the route section SHALL be prepended with a `{ "inbound": ["tun-in"], "action": "sniff" }` rule, and, when DNS is enabled and `dns_hijack` is `Hijack`, a `{ "protocol": "dns", "action": "hijack-dns" }` rule immediately after it. The route section SHALL set `auto_detect_interface: true`.

#### Scenario: sing-box TUN inbound emitted when enabled
- **WHEN** TUN is enabled with sing-box, address `172.19.0.1/30`, MTU 1500, stack system, and strict route on
- **THEN** the generated config inbounds SHALL include a `{ "type": "tun", "auto_route": true, "address": ["172.19.0.1/30"], "mtu": 1500, "stack": "system", "strict_route": true }` entry with no `sniff`/`dns_mode` fields, the route section SHALL include `"auto_detect_interface": true`, and `route.rules` SHALL start with `{ "inbound": ["tun-in"], "action": "sniff" }`

#### Scenario: Excluded routes mapped
- **WHEN** TUN is enabled with `exclude_routes` `["192.168.0.0/16"]`
- **THEN** the sing-box tun inbound SHALL include `"route_exclude_address": ["192.168.0.0/16"]`

#### Scenario: No sing-box TUN inbound when disabled
- **WHEN** TUN is disabled
- **THEN** the generated sing-box config SHALL NOT contain any inbound of type `tun`

### Requirement: Generate xray TUN inbound
When TUN is enabled and the backend is xray, the system SHALL add a native `tun` protocol inbound to the generated config alongside the existing socks/http inbounds, with the configured name, MTU, gateway address(es), DNS, `autoOutboundsInterface: "auto"`, and sniffing enabled. When `dns_hijack` is `Hijack`, the config SHALL additionally contain a `{"protocol": "dns", "tag": "dns-out"}` outbound and a routing rule `{"network": "udp", "port": 53, "outboundTag": "dns-out"}` placed after the `dns-internal` inboundTag rule and before exclusion and user rules; `Native` and `Disabled` SHALL omit both.

#### Scenario: xray TUN inbound emitted when enabled
- **WHEN** TUN is enabled with xray, address `198.18.0.1/30`, and MTU 1500
- **THEN** the generated config inbounds SHALL include a `{ "protocol": "tun", "settings": { "name": "...", "mtu": 1500, "gateway": ["198.18.0.1/30"], "autoOutboundsInterface": "auto" } }` entry with sniffing enabled

#### Scenario: No xray TUN inbound when disabled
- **WHEN** TUN is disabled
- **THEN** the generated xray config SHALL NOT contain any `tun`-protocol inbound

#### Scenario: Application DNS hijacked under Hijack mode
- **WHEN** TUN is enabled with xray and `dns_hijack` is `Hijack`
- **THEN** the config SHALL contain the `dns-out` outbound and the `udp/53 → dns-out` routing rule so TUN-captured plaintext DNS is answered by the built-in resolver

#### Scenario: No hijack under Native or Disabled
- **WHEN** TUN is enabled with xray and `dns_hijack` is `Native` or `Disabled`
- **THEN** the config SHALL contain neither the `dns-out` outbound nor the `udp/53` routing rule

### Requirement: v2ray backend never emits a TUN inbound
When the backend is v2ray, the system SHALL NOT emit a TUN inbound regardless of the persisted `tun.enabled` flag, because v2ray-core has no native TUN support.

#### Scenario: v2ray ignores TUN
- **WHEN** the backend is v2ray and `tun.enabled` is true
- **THEN** the generated v2ray config SHALL contain only the socks and http inbounds and no tun inbound

### Requirement: Exclude traffic from the TUN tunnel
When TUN is enabled, the system SHALL generate backend rules that keep configured
processes and destinations out of the tunnel, mapped to each backend's native
mechanism. Process-name exclusion SHALL be emitted for sing-box only, because
xray cannot match TUN-captured traffic by process. Destination exclusion (CIDR
and domain) SHALL be emitted for both backends. Exclusion rules SHALL take
precedence over the user's routing rules, and excluded DNS SHALL resolve directly
so excluded traffic does not leak through hijacked DNS. The server that answers
for excluded domains SHALL be the first DNS server detoured to `direct` when one
is configured, because a server on the default route resolves through the
tunnel under TUN; only when none is detoured SHALL the first configured server be
used.

#### Scenario: sing-box process-name exclusion
- **WHEN** TUN is enabled with sing-box and `exclude_processes` is `["cloudflared"]`
- **THEN** the sing-box `route.rules` SHALL include, ahead of the user rules, a rule `{ "process_name": ["cloudflared"], "outbound": "direct" }`

#### Scenario: sing-box domain exclusion with direct DNS
- **WHEN** TUN is enabled with sing-box and `exclude_domains` is `["example.com"]` and DNS is enabled
- **THEN** the sing-box `route.rules` SHALL include `{ "domain_suffix": ["example.com"], "outbound": "direct" }` ahead of the user rules, and `dns.rules` SHALL include a matching rule routing those domains to the server detoured to `direct`, or to the first configured server when none is

#### Scenario: xray destination exclusion via the direct outbound
- **WHEN** TUN is enabled with xray and `exclude_routes` is `["104.16.0.0/13"]` and `exclude_domains` is `["example.com"]`
- **THEN** the xray `routing.rules` SHALL include, ahead of the user rules, `{ "type": "field", "ip": ["104.16.0.0/13"], "outboundTag": "direct" }` and `{ "type": "field", "domain": ["example.com"], "outboundTag": "direct" }`, which bypass the tunnel because the direct outbound carries the TUN fwmark

#### Scenario: xray excluded domains resolve directly
- **WHEN** TUN is enabled with xray, `exclude_domains` is `["example.com"]`, and DNS is enabled
- **THEN** the excluded domains SHALL be bound to the `domains` list of the DNS server detoured to `direct`, or of the first configured server when none is, so their resolution does not traverse the tunnel

#### Scenario: No exclusion rules when TUN disabled
- **WHEN** TUN is disabled
- **THEN** neither generator SHALL emit exclusion rules derived from `exclude_processes`, `exclude_domains`, or `exclude_routes`

### Requirement: TUN mode DNS resolution is self-contained
When TUN is enabled, the generated config SHALL NOT depend on the operating-system resolver for any resolution that feeds routing decisions or direct dials. When the DNS feature is disabled in settings, the generator SHALL derive a minimal DNS configuration — a DoH server at an IP-literal endpoint (`https://1.1.1.1/dns-query`) whose queries travel through the first proxy outbound — for the duration of config generation, without mutating settings. For xray this means: a `dns` section with `tag: "dns-internal"` plus a routing rule sending `inboundTag: ["dns-internal"]` to the first proxy outbound ahead of all user rules, and `"domainStrategy": "UseIP"` on the `freedom` direct outbound. For sing-box this means: the `dns` section, `dns.final`, and `route.default_domain_resolver` are emitted with the derived server (detour = first proxy outbound) even though the DNS feature is off.

Static host overrides, cache control and the EDNS client subnet SHALL be emitted on both the derived and the user-configured path, for both backends, so a connect-time host pin reaches the generated config regardless of whether the DNS feature is enabled. For xray, host overrides SHALL be filtered to the address family the query strategy selects, and a domain left with no address of that family SHALL be omitted rather than emitted empty, because xray answers a `hosts` hit authoritatively against a single-family strategy. For sing-box every pinned address SHALL be carried, because the backend applies its strategy after the lookup and would otherwise lose its fallback family.

For sing-box, dial-time name resolution does not consult `dns.rules`, so a pinned hostname reaches a dial only when the outbound names the pin directly. A proxy outbound whose server address is a hostname carried by the host overrides SHALL therefore carry `domain_resolver` naming the `hosts` server. An outbound whose hostname is not pinned, or that is addressed by an IP literal, SHALL NOT name it, because that server answers NXDOMAIN for a name it does not hold rather than falling through.

For xray under TUN the generator SHALL emit bootstrap DNS servers for every name that must resolve before the tunnel carries traffic — each hostname-addressed proxy node and each hostname-addressed DNS server. The bootstrap SHALL be one server object per transport, plain UDP before DoH, each carrying `tag: "dns-direct"`, `skipFallback: true` and a `domains` list scoped to those names, with `finalQuery: true` on the last one only so the pair is tried in order and nothing falls back past it. The routing rules SHALL begin with `{"inboundTag": ["dns-direct"], "outboundTag": "direct"}` so those queries leave through the marked direct outbound instead of the tunnel. When every such address is an IP literal the generator SHALL emit neither the bootstrap server nor the rule. When no server would otherwise be emitted, xray under TUN SHALL fall back to the derived DoH endpoint rather than the operating-system resolver, which bypasses the outbound stack onto an unmarked socket.

#### Scenario: xray TUN with DNS settings off derives a DNS plane
- **WHEN** TUN is enabled, the DNS feature is disabled, and the backend is xray
- **THEN** the generated config SHALL contain a `dns` section with `"tag": "dns-internal"` and a DoH server `https://1.1.1.1/dns-query`, and `routing.rules` SHALL contain `{"inboundTag": ["dns-internal"], "outboundTag": <first proxy tag>}` ahead of all user rules

#### Scenario: Host overrides survive the derived path
- **WHEN** TUN is enabled, the DNS feature is disabled, the backend is xray, and settings carry a host override for the connected node's hostname
- **THEN** the generated `dns` section SHALL contain a `hosts` object mapping that hostname to the override address

#### Scenario: Host overrides are filtered to the query strategy's family
- **WHEN** a host override maps a domain to both an IPv4 and an IPv6 address and the query strategy is IPv4-only
- **THEN** the emitted `hosts` entry SHALL contain only the IPv4 address, and a domain left with no IPv4 address SHALL be absent from `hosts` entirely

#### Scenario: Hostname-addressed node gets direct bootstrap resolvers
- **WHEN** TUN is enabled, the backend is xray, and a proxy node is addressed by hostname
- **THEN** `dns.servers` SHALL begin with a plain-UDP entry and then a DoH entry, both `{"tag": "dns-direct", "domains": ["full:<node hostname>"], "skipFallback": true}`, with `finalQuery: true` on the DoH entry only, and `routing.rules[0]` SHALL be `{"inboundTag": ["dns-direct"], "outboundTag": "direct"}`

#### Scenario: A dead bootstrap transport falls through to the next
- **WHEN** the first bootstrap entry cannot answer
- **THEN** the second SHALL be queried over the same direct route, and the query SHALL NOT fall back onto a resolver that is only reachable through the proxy being resolved

#### Scenario: Hostname-addressed DNS servers are bootstrapped too
- **WHEN** TUN is enabled, the backend is xray, and a configured DNS server is addressed by hostname
- **THEN** that hostname SHALL appear in the bootstrap server's `domains` list

#### Scenario: IP-literal configuration needs no bootstrap
- **WHEN** TUN is enabled, the backend is xray, and every proxy node and DNS server is addressed by an IP literal
- **THEN** the config SHALL contain no `dns-direct` server and no `dns-direct` routing rule

#### Scenario: The direct DNS rule precedes the port-53 hijack
- **WHEN** TUN is enabled, the backend is xray, DNS hijack is on, and a `dns-direct` server is emitted
- **THEN** the `dns-direct` routing rule SHALL appear before the `{"network": "tcp,udp", "port": 53, "outboundTag": "dns-out"}` rule, so a direct plain-UDP resolver is not captured back into the internal resolver

#### Scenario: xray direct outbound never uses the OS resolver under TUN
- **WHEN** TUN is enabled and the backend is xray
- **THEN** the `freedom` outbound SHALL carry `"domainStrategy": "UseIP"`, and SHALL NOT carry it when TUN is disabled

#### Scenario: sing-box TUN with DNS settings off derives a DNS plane
- **WHEN** TUN is enabled, the DNS feature is disabled, and the backend is sing-box
- **THEN** the generated config SHALL contain `dns.servers` with the derived DoH server (detour = first proxy outbound tag), `dns.final` pointing at it, and `route.default_domain_resolver` set

#### Scenario: The pin survives the sing-box derived path
- **WHEN** TUN is enabled, the DNS feature is disabled, the backend is sing-box, and settings carry a host override for the connected node's hostname
- **THEN** `dns.servers` SHALL contain a `hosts` server whose `predefined` maps that hostname to the override address, and `dns.rules` SHALL begin with a rule sending that domain to it

#### Scenario: A pinned proxy outbound resolves from the pin
- **WHEN** the backend is sing-box, a `hosts` server is emitted, and a proxy node's server address is a hostname the overrides carry
- **THEN** that outbound SHALL carry `domain_resolver` naming the `hosts` server, so the dial does not depend on a resolver reachable only through the proxy

#### Scenario: An unpinned outbound is not pointed at the pin
- **WHEN** the backend is sing-box and a proxy node's hostname has no host override, or the configuration has no DNS section at all
- **THEN** that outbound SHALL NOT carry `domain_resolver`

#### Scenario: User-configured DNS is preserved and hardened
- **WHEN** TUN is enabled and the DNS feature is enabled with user servers
- **THEN** the user's servers SHALL be emitted as today, and (xray) the `dns-internal` inboundTag rule SHALL still be present so internal queries traverse the proxy

### Requirement: sing-box rule-sets prefer local cached files
When generating a sing-box config, each referenced GeoIP/GeoSite rule-set SHALL be emitted as `type: "local"` with `format: "binary"` and the absolute path of the cached `.srs` file when that file exists in the geodata cache, and as `type: "remote"` otherwise.

#### Scenario: Cached tag becomes a local rule-set
- **WHEN** `geosite-yandex.srs` exists under the geodata rule-set cache and a routing rule references GeoSite "yandex"
- **THEN** the emitted rule-set entry SHALL be `{"type": "local", "format": "binary", "path": "<cache>/geosite-yandex.srs"}` for tag `geosite-yandex`

#### Scenario: Uncached tag falls back to remote
- **WHEN** no cached `.srs` exists for a referenced tag
- **THEN** the emitted rule-set entry for that tag SHALL be `type: "remote"` with the upstream URL

#### Scenario: Mixed local and remote sets coexist
- **WHEN** some referenced tags are cached and others are not
- **THEN** the config SHALL contain local entries for the cached tags and remote entries for the rest, and `experimental.cache_file` SHALL be enabled while any remote entry is present
