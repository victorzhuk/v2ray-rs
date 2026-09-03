## MODIFIED Requirements

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
