# Spec Delta

## MODIFIED Requirements

### Requirement: Preset server conventions
Each preset's remote server SHALL use DoH protocol with detour `"proxy"` (encrypted, routed through the proxy) and the domestic server SHALL use UDP protocol with detour `"direct"` (fast, routed directly).

#### Scenario: Cloudflare preset servers
- **WHEN** the Cloudflare preset is accessed
- **THEN** it SHALL have a remote server with DoH protocol, address `"1.1.1.1"`, and detour `"proxy"`, and a domestic server with UDP protocol, address `"1.0.0.1"`, and detour `"direct"`

#### Scenario: AdGuard preset servers
- **WHEN** the AdGuard preset is accessed
- **THEN** it SHALL have a remote server with DoH protocol, address `"dns.adguard.com"`, and detour `"proxy"`, and a domestic server with UDP protocol, address `"94.140.14.14"`, and detour `"direct"`

#### Scenario: Yandex preset servers
- **WHEN** the Yandex DNS preset is accessed
- **THEN** it SHALL have a remote server with DoH protocol, address `"common.dot.dns.yandex.net"`, and detour `"proxy"`, and a domestic server with UDP protocol, address `"77.88.8.8"`, and detour `"direct"`
