## MODIFIED Requirements

### Requirement: Generate v2ray-compatible configuration
The system SHALL generate a valid JSON configuration file for v2ray/xray containing inbound, outbound, routing, and DNS sections. When DNS is enabled, the DNS section SHALL reflect the full DNS configuration model including multiple servers, query strategy, hosts, cache settings, and client IP. Inbound `listen` SHALL be taken from `AppSettings::listen_address` (default `127.0.0.1`), and the SOCKS-capable inbound SHALL declare `settings.udp = true`. TLS outbounds SHALL carry `allowInsecure` only for the v2ray backend; for xray, which rejects `allowInsecure: true` as a removed feature, the field SHALL NOT be emitted and a node with certificate verification disabled SHALL fail config generation for that candidate with an error naming the node.

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

#### Scenario: xray TLS outbound omits allowInsecure
- **WHEN** the backend is xray and a TLS node has certificate verification enabled
- **THEN** its `tlsSettings` SHALL NOT contain `allowInsecure`

#### Scenario: xray refuses a node with verification disabled
- **WHEN** the backend is xray and the candidate is a TLS node with certificate verification disabled
- **THEN** config generation for that candidate SHALL fail with an error naming the node and stating that the backend does not support disabled verification, and connection planning SHALL continue with the next candidate

#### Scenario: v2ray keeps allowInsecure
- **WHEN** the backend is v2ray and a TLS node has certificate verification disabled
- **THEN** its `tlsSettings` SHALL contain `"allowInsecure": true`

### Requirement: TUN mode DNS resolution is self-contained
When TUN is enabled, the generated config SHALL NOT depend on the operating-system resolver for any resolution that feeds routing decisions or direct dials. When the DNS feature is disabled in settings, the generator SHALL derive a minimal DNS configuration — a DoH server at an IP-literal endpoint (`https://1.1.1.1/dns-query`) whose queries travel through the first proxy outbound — for the duration of config generation, without mutating settings. For xray this means: a `dns` section with `tag: "dns-internal"` plus a routing rule sending `inboundTag: ["dns-internal"]` to the first proxy outbound ahead of all user rules, and the `freedom` direct outbound resolving through the built-in resolver via `streamSettings.sockopt.domainStrategy`, set from the query strategy like every other dialing outbound; the deprecated `settings.domainStrategy` SHALL NOT be emitted, because current Xray-core copies it over the `sockopt` value. For sing-box this means: the `dns` section, `dns.final`, and `route.default_domain_resolver` are emitted with the derived server (detour = first proxy outbound) even though the DNS feature is off.

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
- **THEN** the `freedom` outbound SHALL carry `streamSettings.sockopt.domainStrategy` of `"UseIPv6"` when the query strategy prefers or requires IPv6 and `"UseIPv4"` otherwise, SHALL NOT carry `settings.domainStrategy`, and SHALL carry neither when TUN is disabled

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

## ADDED Requirements

### Requirement: Generated xray configs load without deprecation warnings
Every xray config the system generates SHALL load on the installed Xray-core without the backend reporting a deprecated or automatically migrated setting, so that a field's removal upstream cannot silently change or break a working configuration. Deprecation notices about a proxy protocol or transport that the user's node itself uses (for example Shadowsocks or WebSocket) are outside this requirement: no generator change can remove them.

#### Scenario: Protocol-level notices are tolerated
- **WHEN** a generated config for a Shadowsocks node or a WebSocket-transport node is checked on Xray-core 26.9.9
- **THEN** the check SHALL succeed, and the only deprecation notices in its output SHALL be the ones naming that protocol or transport

#### Scenario: TUN config on current Xray-core
- **WHEN** an xray TUN config with the DNS feature off and a hostname-addressed REALITY node is checked with `xray run -test` on Xray-core 26.9.9
- **THEN** the check SHALL succeed and its output SHALL contain no line reporting a deprecated setting
