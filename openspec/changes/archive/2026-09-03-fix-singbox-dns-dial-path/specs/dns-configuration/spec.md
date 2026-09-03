## MODIFIED Requirements

### Requirement: DNS server detour per backend
The optional detour field on DNS servers SHALL be honored by the sing-box and xray config generators and ignored by the v2ray generator. Each backend SHALL express only the detour it can start against. sing-box SHALL emit a `detour` field naming the first proxy outbound for any detour other than "direct"; a detour of "direct" SHALL be expressed by omitting the field, because a DNS server carrying no detour is not dispatched through the proxy chain and sing-box refuses to start when a DNS server detours to an outbound that carries no settings. xray has no per-server detour field, so a detour of "direct" SHALL be expressed as a `tag` on the server object plus a routing rule sending that tag to the direct outbound; any other detour value SHALL be ignored, because it names the default route. A server with no detour SHALL keep the default behaviour of traversing the proxy.

#### Scenario: Detour emitted for sing-box
- **WHEN** a DNS server has a detour other than "direct" and the backend is sing-box
- **THEN** the generated server object SHALL include a "detour" field naming the tag of the first proxy outbound

#### Scenario: Direct detour omitted for sing-box
- **WHEN** a DNS server has a detour of "direct" and the backend is sing-box
- **THEN** the generated server object SHALL carry no "detour" field, and the generated configuration SHALL start against the backend rather than being rejected at service start

#### Scenario: Direct detour becomes a tag and a rule for xray
- **WHEN** a DNS server has a detour of "direct" and the backend is xray
- **THEN** the generated server object SHALL carry a tag identifying it as directly routed, and `routing.rules` SHALL contain a rule sending that `inboundTag` to the direct outbound ahead of the internal-resolver rule

#### Scenario: Detour ignored for v2ray/xray
- **WHEN** a DNS server has a detour the backend cannot express — any value for v2ray, or any value other than "direct" for xray
- **THEN** the generated DNS config SHALL NOT include any detour-related field or tag for that server, and the query SHALL follow the backend's default routing
