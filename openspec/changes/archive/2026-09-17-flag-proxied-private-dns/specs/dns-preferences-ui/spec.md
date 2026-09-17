## ADDED Requirements

### Requirement: Warning for private DNS servers routed through the proxy
The DNS server dialog SHALL show a non-blocking inline warning while the entered address, detour, and active backend make the server flagged as a private DNS server routed through the proxy, stating that queries will reach the proxy server's network and suggesting detour `direct`. The server's row in the list and in the primary section SHALL show the same warning. Saving SHALL remain allowed.

#### Scenario: Warning while editing
- **WHEN** the active backend is sing-box and the user enters address `127.0.0.1` with detour `proxy`
- **THEN** the dialog SHALL show the warning and the Save response SHALL stay enabled

#### Scenario: Warning clears on direct detour
- **WHEN** the user changes that server's detour to `direct`
- **THEN** the warning SHALL disappear

#### Scenario: Saved flagged server marked
- **WHEN** a saved server is flagged for the active backend
- **THEN** its row SHALL show the warning text

### Requirement: Strategy options reflect the backend
The IP strategy selector SHALL keep its four options and stored values, and when the active backend is xray or v2ray SHALL show a note that "Prefer IPv4" and "Prefer IPv6" query only the preferred address family on that backend. For sing-box no note SHALL be shown.

#### Scenario: xray strategy note
- **WHEN** the active backend is xray and the DNS page is shown
- **THEN** the strategy row SHALL show the note that Prefer options use only the preferred family

#### Scenario: sing-box shows no note
- **WHEN** the active backend is sing-box
- **THEN** the strategy row SHALL show no family note