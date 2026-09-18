## ADDED Requirements

### Requirement: Domain matchers keep their meaning on every backend
Every generator SHALL emit suffix-meaning domain conditions with backend matchers that match the named domain and all its subdomains, in routing rules, DNS rules, derived DNS server domain lists, and TUN exclusions. Xray and v2ray SHALL emit `domain:<name>`; sing-box SHALL emit `domain_suffix` with `<name>`. A leading `*.` SHALL be removed before emission. A domain keyword SHALL remain a substring matcher, and a full domain SHALL remain an exact matcher. No emitted suffix value SHALL contain `*`.

#### Scenario: Wildcard domain pattern on xray
- **WHEN** a routing rule has domain pattern `*.google.com` and the backend is xray or v2ray
- **THEN** the routing rule SHALL contain `"domain": ["domain:google.com"]`

#### Scenario: Wildcard domain pattern on sing-box
- **WHEN** a routing rule has domain pattern `*.google.com` and the backend is sing-box
- **THEN** the route rule SHALL contain `"domain_suffix": ["google.com"]`

#### Scenario: Derived DNS domains use the same matcher
- **WHEN** DNS is enabled with auto-derived rules and a direct routing rule has domain pattern `*.example.com`
- **THEN** xray's domestic server SHALL contain `domain:example.com` and sing-box's domestic DNS rule SHALL contain `domain_suffix` `example.com`

#### Scenario: Keyword stays a substring matcher
- **WHEN** a routing rule has domain keyword `sina`
- **THEN** xray and v2ray SHALL emit `"domain": ["sina"]` and sing-box SHALL emit `"domain_keyword": ["sina"]`

## MODIFIED Requirements

### Requirement: Exclude traffic from the TUN tunnel
When TUN is enabled, the system SHALL generate backend rules that keep configured processes and destinations out of the tunnel, mapped to each backend's native mechanism. Process-name exclusion SHALL be emitted for sing-box only. Destination exclusion (CIDR and domain) SHALL be emitted for both backends and SHALL precede user routing rules. Excluded DNS SHALL resolve directly through the first DNS server detoured to `direct`, or the first configured server when none is detoured. Excluded domains SHALL match the named domain and its subdomains: sing-box SHALL emit `domain_suffix`; xray SHALL emit `domain:<name>` in routing rules and DNS server `domains` lists, never the unprefixed name.

#### Scenario: xray excluded domains resolve directly
- **WHEN** TUN is enabled with xray, `exclude_domains` is `["example.com"]`, and DNS is enabled
- **THEN** `domain:example.com` SHALL be bound to the DNS server detoured to `direct`, or to the first configured server when none is detoured

#### Scenario: Xray excluded domain does not match by substring
- **WHEN** TUN is enabled with xray and `exclude_domains` is `["wb.ru"]`
- **THEN** no generated routing rule or DNS server `domains` list SHALL contain the unprefixed string `wb.ru`

#### Scenario: sing-box process-name exclusion
- **WHEN** TUN is enabled with sing-box and `exclude_processes` is `["cloudflared"]`
- **THEN** sing-box `route.rules` SHALL include, ahead of user rules, `{ "process_name": ["cloudflared"], "outbound": "direct" }`

#### Scenario: sing-box domain exclusion with direct DNS
- **WHEN** TUN is enabled with sing-box, `exclude_domains` is `["example.com"]`, and DNS is enabled
- **THEN** `route.rules` SHALL include `{ "domain_suffix": ["example.com"], "outbound": "direct" }` ahead of user rules and `dns.rules` SHALL route those domains to the direct-detoured server, or the first configured server when none is detoured

#### Scenario: xray destination exclusion via the direct outbound
- **WHEN** TUN is enabled with xray, `exclude_routes` is `["104.16.0.0/13"]`, and `exclude_domains` is `["example.com"]`
- **THEN** `routing.rules` SHALL include, ahead of user rules, direct rules for the CIDR and `domain:example.com`

#### Scenario: No exclusion rules when TUN disabled
- **WHEN** TUN is disabled
- **THEN** neither generator SHALL emit exclusions derived from `exclude_processes`, `exclude_domains`, or `exclude_routes`
