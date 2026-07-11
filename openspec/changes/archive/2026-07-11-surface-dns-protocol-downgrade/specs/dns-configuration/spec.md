## ADDED Requirements

### Requirement: Backend DNS protocol compatibility matrix
The system SHALL define per-backend DNS protocol support normatively: sing-box supports UDP, TCP, DoH, DoT, DoQ, and H3 natively; xray supports all except H3; v2ray supports only UDP, TCP, and DoH. Protocols a backend does not support SHALL be downgraded to DoH at config-generation time. The compatibility and downgrade mapping SHALL live in one core function that both the config generators and the UI consult.

#### Scenario: Downgrade mapping is single-sourced
- **WHEN** the UI or a config generator needs to know a protocol's effective form on a backend
- **THEN** both SHALL consult the same core compatibility function, so UI messaging and generated configs cannot drift

#### Scenario: sing-box passes protocols through natively
- **WHEN** a DNS server is configured with any supported protocol and the backend is sing-box
- **THEN** the generated config SHALL use the configured protocol without downgrade
