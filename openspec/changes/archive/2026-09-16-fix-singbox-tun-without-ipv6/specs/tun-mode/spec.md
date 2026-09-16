## ADDED Requirements

### Requirement: TUN starts on hosts with kernel IPv6 disabled
The system SHALL start TUN connections on a host whose kernel has no IPv6 support (booted with `ipv6.disable=1`). For sing-box, the connection's generated config SHALL set `strict_route: false` on such a host regardless of the persisted setting, because sing-box's strict route adds an IPv6 policy rule the kernel rejects, and SHALL write one notice line to the process log stream stating that kernel IPv6 is disabled and strict route was turned off for the session. The persisted `strict_route` setting SHALL NOT change. When an IPv6 tunnel address is configured on such a host, a TUN connection for either backend SHALL fail before any backend is spawned, with an error stating that the kernel has IPv6 disabled and that the IPv6 tunnel address must be cleared.

#### Scenario: sing-box strict route on an IPv6-less host
- **WHEN** a sing-box TUN connection starts with `strict_route` on, no IPv6 tunnel address, and kernel IPv6 disabled
- **THEN** the generated tun inbound SHALL contain `"strict_route": false`, the process log SHALL contain one notice naming disabled kernel IPv6, and the persisted settings SHALL still have `strict_route` on

#### Scenario: sing-box strict route on a host with IPv6
- **WHEN** a sing-box TUN connection starts with `strict_route` on and kernel IPv6 available (including with `net.ipv6.conf.all.disable_ipv6=1`)
- **THEN** the generated tun inbound SHALL contain `"strict_route": true` and no notice SHALL be logged

#### Scenario: IPv6 tunnel address on an IPv6-less host
- **WHEN** TUN is enabled with an IPv6 tunnel address, the backend is sing-box or xray, and kernel IPv6 is disabled
- **THEN** Connect SHALL fail without spawning a backend or trying any candidate, and the user SHALL see an error naming disabled kernel IPv6 and the IPv6 tunnel address setting
