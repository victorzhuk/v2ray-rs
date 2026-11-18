# Spec Delta

## ADDED Requirements

### Requirement: Xray JSON DNS server schemes and pseudo-servers import accurately
When importing an Xray JSON `dns` section, `https+local://`, `quic+local://`, and `tcp+local://` server URLs SHALL map to `Doh`, `Doq`, and `Tcp` protocols with `detour = "direct"`. Built-in `localhost` and `fakedns` pseudo-servers and DoH/H3 URLs carrying non-standard paths other than `/dns-query` SHALL be skipped with a surfaced warning rather than coerced into plain UDP port `53` servers.

#### Scenario: Local-mode Xray DNS URLs preserve transport and direct detour
- **WHEN** an Xray JSON bundle contains `"https+local://1.1.1.1/dns-query"` and `"quic+local://dns.adguard.com:853"`
- **THEN** the importer SHALL produce a `Doh` server (`1.1.1.1`, detour `"direct"`) and a `Doq` server (`dns.adguard.com`, port `853`, detour `"direct"`)

#### Scenario: Pseudo-servers and custom DoH paths are skipped with warnings
- **WHEN** an Xray JSON bundle contains `"localhost"`, `"fakedns"`, or `"https://dns.example.com/custom-endpoint"`
- **THEN** the importer SHALL skip those DNS entries, emit a warning naming each skipped server, and SHALL NOT emit a `Udp` server on port `53` for them
