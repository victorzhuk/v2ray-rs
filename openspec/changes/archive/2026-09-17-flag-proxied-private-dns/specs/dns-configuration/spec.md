## ADDED Requirements

### Requirement: Private DNS servers routed through the proxy are flagged
The system SHALL identify a DNS server whose address is an IP literal in a loopback (`127.0.0.0/8`, `::1`), private (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`), link-local (`169.254.0.0/16`, `fe80::/10`), or unique-local (`fc00::/7`) range and whose queries the generated config sends through the proxy: on sing-box, a server with a detour other than "direct"; on xray, any server not expressed as directly routed (a "direct" detour under TUN). Such a server reaches the proxy server's network, not the user's. The system SHALL NOT reject such a server, because a resolver on the proxy server's own network is a valid setup; config generation SHALL log one warning per flagged server naming its tag and address. Hostname-addressed servers and the v2ray backend SHALL NOT be flagged.

#### Scenario: Loopback server with proxy detour on sing-box
- **WHEN** the backend is sing-box and a server `domestic` is `udp 127.0.0.1` with detour `proxy`
- **THEN** the server SHALL be flagged, config generation SHALL succeed, and a warning naming `domestic` and `127.0.0.1` SHALL be logged

#### Scenario: Loopback server without direct routing on xray
- **WHEN** the backend is xray with TUN enabled and a server is `udp 127.0.0.1` with detour `proxy` or no detour
- **THEN** the server SHALL be flagged

#### Scenario: Direct private server is not flagged
- **WHEN** a server is `udp 192.168.1.1` with detour `direct` and the backend is sing-box, or xray with TUN enabled
- **THEN** the server SHALL NOT be flagged

#### Scenario: Public server is not flagged
- **WHEN** a server is `udp 1.1.1.1` with detour `proxy`
- **THEN** the server SHALL NOT be flagged