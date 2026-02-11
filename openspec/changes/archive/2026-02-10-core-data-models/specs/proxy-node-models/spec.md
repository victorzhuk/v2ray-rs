# Spec: proxy-node-models

## ADDED Requirements

### Requirement: Proxy protocol type representation
The system SHALL define Rust types for VLESS, VMess, Shadowsocks, and Trojan proxy protocols. Each protocol type SHALL contain all fields required to generate a valid backend configuration entry.

#### Scenario: VLESS node representation
- **WHEN** a VLESS proxy node is created
- **THEN** it SHALL contain: address, port, uuid, encryption, flow, transport settings (tcp/ws/grpc/h2), TLS settings, and an optional remark/alias

#### Scenario: VMess node representation
- **WHEN** a VMess proxy node is created
- **THEN** it SHALL contain: address, port, uuid, alterId, security, transport settings, TLS settings, and an optional remark/alias

#### Scenario: Shadowsocks node representation
- **WHEN** a Shadowsocks proxy node is created
- **THEN** it SHALL contain: address, port, method (cipher), password, and an optional remark/alias

#### Scenario: Trojan node representation
- **WHEN** a Trojan proxy node is created
- **THEN** it SHALL contain: address, port, password, TLS settings, transport settings, and an optional remark/alias

### Requirement: Proxy node enum
The system SHALL use a Rust enum to represent proxy nodes, with one variant per supported protocol. The enum SHALL support serde serialization with tagged representation.

#### Scenario: Exhaustive protocol matching
- **WHEN** code handles a proxy node
- **THEN** the Rust compiler SHALL enforce handling of all protocol variants via exhaustive match

#### Scenario: Serialization round-trip
- **WHEN** a proxy node is serialized to JSON and deserialized back
- **THEN** the result SHALL be identical to the original

### Requirement: Subscription model
The system SHALL define a subscription type containing: unique ID, name, source URL or file path, list of parsed proxy nodes, last update timestamp, auto-update interval, and enabled/disabled status.

#### Scenario: Subscription with multiple nodes
- **WHEN** a subscription is parsed
- **THEN** it SHALL contain zero or more proxy nodes, each individually enable-able/disable-able

### Requirement: Routing rule model
The system SHALL define routing rule types for: GeoIP country match, GeoSite category match, domain pattern match, and IP CIDR match. Each rule SHALL have an associated action (proxy, direct, block) and a priority/order value.

#### Scenario: GeoIP rule
- **WHEN** a GeoIP routing rule is created with country code "RU" and action "direct"
- **THEN** it SHALL represent that traffic matching Russian IPs goes direct

#### Scenario: Rule ordering
- **WHEN** multiple routing rules exist
- **THEN** they SHALL be orderable by priority, with lower values evaluated first

### Requirement: Backend configuration model
The system SHALL define a backend configuration type containing: selected backend (v2ray/xray/sing-box), binary path, config output directory, and backend-specific settings.

#### Scenario: Backend selection persistence
- **WHEN** the user selects xray as their backend
- **THEN** the selection SHALL persist across app restarts

### Requirement: Application settings model
The system SHALL define an application settings type containing: local proxy ports (SOCKS5, HTTP), auto-update preferences, UI preferences, selected language, and general app behavior flags.

#### Scenario: Default settings
- **WHEN** the app launches for the first time with no existing config
- **THEN** it SHALL use sensible defaults (SOCKS5 port 1080, HTTP port 1081, English language)
