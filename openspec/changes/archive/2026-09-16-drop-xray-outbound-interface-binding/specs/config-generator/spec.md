## MODIFIED Requirements

### Requirement: Generate xray TUN inbound
When TUN is enabled and the backend is xray, the system SHALL add a native `tun` protocol inbound to the generated config alongside the existing socks/http inbounds, with the configured name, MTU, gateway address(es), DNS, and sniffing enabled; the inbound SHALL NOT set `autoOutboundsInterface`. When `dns_hijack` is `Hijack`, the config SHALL additionally contain a `{"protocol": "dns", "tag": "dns-out"}` outbound and a routing rule `{"network": "udp", "port": 53, "outboundTag": "dns-out"}` placed after the `dns-internal` inboundTag rule and before exclusion and user rules; `Native` and `Disabled` SHALL omit both.

#### Scenario: xray TUN inbound emitted when enabled
- **WHEN** TUN is enabled with xray, address `198.18.0.1/30`, and MTU 1500
- **THEN** the generated config inbounds SHALL include a `{ "protocol": "tun", "settings": { "name": "...", "mtu": 1500, "gateway": ["198.18.0.1/30"] } }` entry with sniffing enabled and no `autoOutboundsInterface` key

#### Scenario: No xray TUN inbound when disabled
- **WHEN** TUN is disabled
- **THEN** the generated xray config SHALL NOT contain any `tun`-protocol inbound

#### Scenario: Application DNS hijacked under Hijack mode
- **WHEN** TUN is enabled with xray and `dns_hijack` is `Hijack`
- **THEN** the config SHALL contain the `dns-out` outbound and the `udp/53 → dns-out` routing rule so TUN-captured plaintext DNS is answered by the built-in resolver

#### Scenario: No hijack under Native or Disabled
- **WHEN** TUN is enabled with xray and `dns_hijack` is `Native` or `Disabled`
- **THEN** the config SHALL contain neither the `dns-out` outbound nor the `udp/53` routing rule
