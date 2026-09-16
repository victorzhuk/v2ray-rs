## MODIFIED Requirements

### Requirement: Outbound loop prevention
The system SHALL configure each backend so the backend's own outbound traffic bypasses the TUN interface and does not loop. For xray, loop prevention SHALL rely on the fwmark carried by every dialing outbound and the route helper's policy rule sending marked traffic to the `main` table; the generated config SHALL NOT bind outbound sockets to a fixed or auto-detected interface, so xray's egress follows every route in `main`, including more-specific routes through other interfaces such as a VPN.

#### Scenario: sing-box loop prevention
- **WHEN** a sing-box TUN config is generated
- **THEN** the route section SHALL set `auto_detect_interface: true`

#### Scenario: xray loop prevention
- **WHEN** an xray TUN config is generated
- **THEN** every outbound other than `blackhole` and `dns` SHALL set `streamSettings.sockopt.mark` to 255, and the tun inbound settings SHALL NOT contain `autoOutboundsInterface`

#### Scenario: xray direct egress follows VPN routes
- **WHEN** xray TUN is connected, a VPN interface holds a more-specific route such as `10.0.0.0/8`, and traffic to an address in that range is routed by xray to `direct`
- **THEN** that traffic SHALL leave through the VPN interface, not the default-route interface
