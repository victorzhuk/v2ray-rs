## ADDED Requirements

### Requirement: Exclude traffic from the TUN tunnel
When TUN is enabled, the system SHALL generate backend rules that keep configured
processes and destinations out of the tunnel, mapped to each backend's native
mechanism. Process-name exclusion SHALL be emitted for sing-box only, because
xray cannot match TUN-captured traffic by process. Destination exclusion (CIDR
and domain) SHALL be emitted for both backends. Exclusion rules SHALL take
precedence over the user's routing rules, and excluded DNS SHALL resolve directly
so excluded traffic does not leak through hijacked DNS.

#### Scenario: sing-box process-name exclusion
- **WHEN** TUN is enabled with sing-box and `exclude_processes` is `["cloudflared"]`
- **THEN** the sing-box `route.rules` SHALL include, ahead of the user rules, a rule `{ "process_name": ["cloudflared"], "outbound": "direct" }`

#### Scenario: sing-box domain exclusion with direct DNS
- **WHEN** TUN is enabled with sing-box and `exclude_domains` is `["example.com"]` and DNS is enabled
- **THEN** the sing-box `route.rules` SHALL include `{ "domain_suffix": ["example.com"], "outbound": "direct" }` ahead of the user rules, and `dns.rules` SHALL include a matching rule routing those domains to a direct (non-detour) server

#### Scenario: xray destination exclusion via the direct outbound
- **WHEN** TUN is enabled with xray and `exclude_routes` is `["104.16.0.0/13"]` and `exclude_domains` is `["example.com"]`
- **THEN** the xray `routing.rules` SHALL include, ahead of the user rules, `{ "type": "field", "ip": ["104.16.0.0/13"], "outboundTag": "direct" }` and `{ "type": "field", "domain": ["example.com"], "outboundTag": "direct" }`, which bypass the tunnel because the direct outbound carries the TUN fwmark

#### Scenario: xray excluded domains resolve directly
- **WHEN** TUN is enabled with xray, `exclude_domains` is `["example.com"]`, and DNS is enabled
- **THEN** the excluded domains SHALL be bound to xray's direct/domestic DNS server (its `domains` list) so their resolution does not traverse the tunnel

#### Scenario: No exclusion rules when TUN disabled
- **WHEN** TUN is disabled
- **THEN** neither generator SHALL emit exclusion rules derived from `exclude_processes`, `exclude_domains`, or `exclude_routes`
