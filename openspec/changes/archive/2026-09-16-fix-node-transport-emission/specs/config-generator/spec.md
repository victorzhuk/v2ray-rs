## ADDED Requirements

### Requirement: WebSocket host is carried on every backend
For a WebSocket node, every generator SHALL send the node's host as the HTTP `Host` of the upgrade request. When the node has custom headers without a `Host` entry (compared case-insensitively) and a host is set, the host SHALL be added as `Host`; a `Host` entry already present in the headers SHALL win over the node host. For xray the resulting `Host` SHALL be emitted in the `wsSettings.host` field, as it already is when the host comes from the headers.

#### Scenario: Headers without Host on v2ray
- **WHEN** the backend is v2ray and a WebSocket node has host `cdn.example.com` and headers `{"User-Agent": "x"}`
- **THEN** `wsSettings.headers` SHALL contain `"Host": "cdn.example.com"` and `"User-Agent": "x"`

#### Scenario: Headers without Host on xray
- **WHEN** the backend is xray and a WebSocket node has host `cdn.example.com` and headers `{"User-Agent": "x"}`
- **THEN** `wsSettings.host` SHALL be `cdn.example.com` and `wsSettings.headers` SHALL contain `"User-Agent": "x"` and no `Host`

#### Scenario: Explicit Host header wins
- **WHEN** a WebSocket node has host `cdn.example.com` and headers `{"host": "front.example.com"}`
- **THEN** every backend SHALL send `front.example.com` as the Host

### Requirement: v2ray backend refuses xray-only node features
When the backend is v2ray, config generation for a candidate SHALL fail with an error naming the node when the node uses REALITY security or the XHTTP transport — naming REALITY for the former, and stating that the backend does not support the node's transport for the latter — because both are Xray-core features, and connection planning SHALL continue with the next candidate. The v2ray Real Delay probe SHALL leave such nodes out of the probe batch instead of failing it. xray and sing-box behavior SHALL NOT change.

#### Scenario: REALITY node on v2ray
- **WHEN** the backend is v2ray and the candidate is a VLESS node with REALITY enabled
- **THEN** config generation SHALL fail with an error naming the node and REALITY, and no `realitySettings` SHALL be written

#### Scenario: XHTTP node on v2ray
- **WHEN** the backend is v2ray and the candidate uses the XHTTP transport
- **THEN** config generation SHALL fail with an error naming the node and stating that the backend does not support its transport

#### Scenario: Probe batch with a REALITY node on v2ray
- **WHEN** a v2ray Real Delay probe covers a REALITY node and a plain TLS node
- **THEN** the probe config SHALL contain an outbound for the TLS node only, tagged with its original batch index

#### Scenario: xray keeps REALITY and XHTTP
- **WHEN** the backend is xray and the candidate uses REALITY or XHTTP
- **THEN** the config SHALL be generated as before
