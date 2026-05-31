# Spec: Observatory Real Delay

## Purpose
Defines how v2ray-rs measures real proxy latency for xray and v2ray backends using their native ObservatoryService gRPC APIs, without implementing any proxy protocols in Rust.

## Requirements

### Requirement: xray Real Delay uses ObservatoryService
The system SHALL support Real Delay probes for xray by spawning an isolated probe backend with xray-compatible outbounds, enabling xray observatory or burst observatory for the probed outbound tags, and reading measured delays from `xray.core.app.observatory.command.ObservatoryService/GetOutboundStatus` over the probe backend's loopback gRPC API.

#### Scenario: xray observatory results map by outbound tag
- **WHEN** the selected backend is xray and the user runs Real Delay for N subscription nodes
- **THEN** the probe config SHALL tag every tested outbound as `probe-<index>`
- **AND** the app SHALL poll `GetOutboundStatus` until observed statuses include those tags or the session deadline expires
- **AND** each returned `OutboundStatus.delay` SHALL be stored against the matching node index when `alive = true` and `delay > 0`

#### Scenario: xray partial observatory result
- **WHEN** xray returns delay statuses for only some probed `probe-<index>` tags before the session deadline
- **THEN** the app SHALL persist successful delay values for returned tags
- **AND** the app SHALL leave missing, failed, or zero-delay tags as unknown Real Delay samples

### Requirement: v2ray Real Delay uses v2fly ObservatoryService
The system SHALL support Real Delay probes for v2fly/v2ray-core builds that expose observatory by spawning an isolated probe backend with v2ray-compatible outbounds, enabling v2ray observatory or burst observatory for the probed outbound tags, and reading measured delays from `v2ray.core.app.observatory.command.ObservatoryService/GetOutboundStatus` over the probe backend's loopback gRPC API.

#### Scenario: v2ray observatory results map by outbound tag
- **WHEN** the selected backend is v2ray and the installed binary exposes `ObservatoryService`
- **THEN** Real Delay SHALL query the v2fly observatory gRPC service instead of the sing-box Clash API
- **AND** the app SHALL map each response status by `outbound_tag = "probe-<index>"` to the corresponding tested node

#### Scenario: v2ray without observatory remains unsupported
- **WHEN** the selected v2ray binary rejects the observatory probe config or the observatory gRPC service is unavailable
- **THEN** the Real Delay session SHALL return no samples for that run
- **AND** the UI SHALL report that Real Delay requires a v2ray build with observatory support

### Requirement: Observatory probe config is isolated and loopback-only
The system SHALL generate xray and v2ray observatory probe configs that run independently from the user-facing backend process, bind the API inbound only to `127.0.0.1:<ephemeral_port>`, expose no SOCKS/HTTP user-facing inbound, and route the API inbound to the backend's API outbound tag.

#### Scenario: xray/v2ray probe config has no user proxy inbound
- **WHEN** the app generates an observatory Real Delay config for xray or v2ray
- **THEN** the config SHALL include only the loopback API inbound needed for gRPC access
- **AND** it SHALL NOT expose the user's configured SOCKS or HTTP proxy ports

#### Scenario: API service list includes observatory
- **WHEN** the app generates an observatory Real Delay config for xray or v2ray
- **THEN** the config SHALL enable the backend API service required for `ObservatoryService`
- **AND** the API inbound SHALL be routed to the API outbound tag without affecting tested proxy outbounds

### Requirement: Observatory polling is bounded and non-blocking
The system SHALL poll xray/v2ray observatory status asynchronously with a bounded deadline derived from `RealDelaySettings::timeout_ms`, SHALL keep the subscriptions UI responsive while polling, and SHALL always stop the ephemeral probe backend after success, timeout, or error.

#### Scenario: API port accepts TCP before gRPC service is ready
- **WHEN** the probe backend accepts a TCP connection on the API port but `GetOutboundStatus` is not yet callable
- **THEN** the app SHALL continue polling until the observatory service responds or the session deadline expires

#### Scenario: observatory polling times out
- **WHEN** no usable observatory delay status arrives before the session deadline
- **THEN** the app SHALL stop the probe backend
- **AND** the Real Delay result for each unfinished node SHALL remain unknown
- **AND** the UI SHALL show a concise diagnostic rather than hanging

#### Scenario: user-facing connection is running during observatory probe
- **WHEN** the user runs xray/v2ray Real Delay while the normal proxy backend is connected
- **THEN** the app SHALL use a separate `ProbeRunner` process
- **AND** it SHALL NOT restart, mutate, or stop the user-facing backend process

### Requirement: Observatory client does not implement proxy protocols
The system SHALL use backend-native xray/v2ray observatory APIs only for measuring Real Delay. It SHALL NOT add Rust implementations of VLESS, VMess, Trojan, Shadowsocks, REALITY, or backend transport protocols for this feature.

#### Scenario: measured request is performed by backend binary
- **WHEN** an xray/v2ray Real Delay session runs
- **THEN** the outbound proxy protocol handshake and HTTP probe request SHALL be performed by the installed backend binary
- **AND** v2ray-rs SHALL only generate config, start/stop the probe backend, call the observatory API, and persist mapped results
