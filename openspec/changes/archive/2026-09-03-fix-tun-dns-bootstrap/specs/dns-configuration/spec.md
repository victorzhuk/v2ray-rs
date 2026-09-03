## RENAMED Requirements

- FROM: `### Requirement: Detour is sing-box only`
- TO: `### Requirement: DNS server detour per backend`

## MODIFIED Requirements

### Requirement: DNS server detour per backend
The optional detour field on DNS servers SHALL be honored by the sing-box and xray config generators and ignored by the v2ray generator. sing-box SHALL emit it as a `detour` field on the server object. xray has no per-server detour field, so a detour of "direct" SHALL be expressed as a `tag` on the server object plus a routing rule sending that tag to the direct outbound; any other detour value SHALL be ignored, because it names the default route. A server with no detour SHALL keep the default behaviour of traversing the proxy.

#### Scenario: Detour emitted for sing-box
- **WHEN** a DNS server has a detour set and the backend is sing-box
- **THEN** the generated server object SHALL include a "detour" field: "direct" passes through unchanged, and any other value resolves to the tag of the first proxy outbound

#### Scenario: Direct detour becomes a tag and a rule for xray
- **WHEN** a DNS server has a detour of "direct" and the backend is xray
- **THEN** the generated server object SHALL carry a tag identifying it as directly routed, and `routing.rules` SHALL contain a rule sending that `inboundTag` to the direct outbound ahead of the internal-resolver rule

#### Scenario: Detour ignored for v2ray/xray
- **WHEN** a DNS server has a detour the backend cannot express — any value for v2ray, or any value other than "direct" for xray
- **THEN** the generated DNS config SHALL NOT include any detour-related field or tag for that server, and the query SHALL follow the backend's default routing

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
