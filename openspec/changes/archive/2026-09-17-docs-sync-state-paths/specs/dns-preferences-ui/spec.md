## ADDED Requirements

### Requirement: Detour applicability is explained per backend
The detour control in the DNS server dialog SHALL state its effect for the selected backend. For xray it SHALL state that only `direct` has an effect and only while TUN is enabled. For sing-box it SHALL state that `proxy` sends the server through the proxy and `direct` dials it directly.

#### Scenario: xray note
- **WHEN** the backend is xray and the user opens the DNS server dialog
- **THEN** the detour row SHALL state that only `direct` applies, and only with TUN enabled

#### Scenario: sing-box note
- **WHEN** the backend is sing-box and the user opens the DNS server dialog
- **THEN** the detour row SHALL state that `proxy` routes the server through the proxy and `direct` dials it directly
