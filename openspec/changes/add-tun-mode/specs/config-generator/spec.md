## ADDED Requirements

### Requirement: Generate sing-box TUN inbound
When TUN is enabled and the backend is sing-box, the system SHALL add a native `tun` inbound to the generated config alongside the existing `mixed`/`http` inbounds. The inbound SHALL set `auto_route: true`, the configured interface name, address(es), and MTU, and SHALL map the advanced settings: `stack`, `strict_route`, `dns_hijack` → `dns_mode`, and `exclude_routes` → `route_exclude_address`. The route section SHALL set `auto_detect_interface: true`.

#### Scenario: sing-box TUN inbound emitted when enabled
- **WHEN** TUN is enabled with sing-box, address `172.19.0.1/30`, MTU 1500, stack system, and strict route on
- **THEN** the generated config inbounds SHALL include a `{ "type": "tun", "auto_route": true, "address": ["172.19.0.1/30"], "mtu": 1500, "stack": "system", "strict_route": true }` entry, and the route section SHALL include `"auto_detect_interface": true`

#### Scenario: Excluded routes mapped
- **WHEN** TUN is enabled with `exclude_routes` `["192.168.0.0/16"]`
- **THEN** the sing-box tun inbound SHALL include `"route_exclude_address": ["192.168.0.0/16"]`

#### Scenario: No sing-box TUN inbound when disabled
- **WHEN** TUN is disabled
- **THEN** the generated sing-box config SHALL NOT contain any inbound of type `tun`

### Requirement: Generate xray TUN inbound
When TUN is enabled and the backend is xray, the system SHALL add a native `tun` protocol inbound to the generated config alongside the existing socks/http inbounds, with the configured name, MTU, gateway address(es), DNS, `autoOutboundsInterface: "auto"`, and sniffing enabled.

#### Scenario: xray TUN inbound emitted when enabled
- **WHEN** TUN is enabled with xray, address `198.18.0.1/30`, and MTU 1500
- **THEN** the generated config inbounds SHALL include a `{ "protocol": "tun", "settings": { "name": "...", "mtu": 1500, "gateway": ["198.18.0.1/30"], "autoOutboundsInterface": "auto" } }` entry with sniffing enabled

#### Scenario: No xray TUN inbound when disabled
- **WHEN** TUN is disabled
- **THEN** the generated xray config SHALL NOT contain any `tun`-protocol inbound

### Requirement: v2ray backend never emits a TUN inbound
When the backend is v2ray, the system SHALL NOT emit a TUN inbound regardless of the persisted `tun.enabled` flag, because v2ray-core has no native TUN support.

#### Scenario: v2ray ignores TUN
- **WHEN** the backend is v2ray and `tun.enabled` is true
- **THEN** the generated v2ray config SHALL contain only the socks and http inbounds and no tun inbound
