## ADDED Requirements

### Requirement: Import subscription from URL
The system SHALL report partial parse failures while still importing valid nodes, and it SHALL avoid persisting a new subscription when no valid nodes are produced.

#### Scenario: Partial success import
- **WHEN** the source contains valid proxy URIs and invalid URIs together
- **THEN** the system SHALL import the valid nodes, persist the subscription, and surface which entries were skipped

#### Scenario: No valid nodes
- **WHEN** a new import source yields zero valid proxy nodes
- **THEN** the system SHALL report the failure and SHALL NOT persist an empty subscription

### Requirement: Import subscription from file
The system SHALL allow file-based imports from onboarding and the main subscriptions page.

#### Scenario: File path entered during onboarding
- **WHEN** the user provides a local subscription file path during onboarding
- **THEN** the onboarding flow SHALL create a file-backed subscription source instead of requiring a URL

### Requirement: Parse VLESS URI
The system SHALL parse `vless://` URIs into VLESS proxy node configurations.

#### Scenario: Standard VLESS URI
- **WHEN** given `vless://uuid@host:port?type=ws&security=tls&sni=example.com#remark`
- **THEN** the system SHALL extract uuid, host, port, transport type, TLS settings, SNI, and remark

### Requirement: Parse VMess URI
The system SHALL parse `vmess://` URIs (base64-encoded JSON) into VMess proxy node configurations.

#### Scenario: Standard VMess URI
- **WHEN** given `vmess://` followed by base64-encoded JSON with fields (v, ps, add, port, id, aid, net, type, host, path, tls)
- **THEN** the system SHALL extract all fields into a VMess node configuration

### Requirement: Parse Shadowsocks URI
The system SHALL parse `ss://` URIs (SIP002 format) into Shadowsocks proxy node configurations.

#### Scenario: SIP002 format
- **WHEN** given `ss://base64(method:password)@host:port#remark`
- **THEN** the system SHALL extract method, password, host, port, and remark

### Requirement: Parse Trojan URI
The system SHALL parse `trojan://` URIs into Trojan proxy node configurations.

#### Scenario: Standard Trojan URI
- **WHEN** given `trojan://password@host:port?sni=example.com#remark`
- **THEN** the system SHALL extract password, host, port, SNI, and remark

### Requirement: Subscription metadata storage
The system SHALL store subscription metadata: unique ID, user-given name, source URL/path, last update timestamp, node count, and enabled status.

#### Scenario: Multiple subscriptions
- **WHEN** the user imports three different subscription URLs
- **THEN** all three SHALL be stored independently with their own metadata and node lists

### Requirement: Per-node enable/disable
The system SHALL allow enabling/disabling individual proxy nodes within a subscription.

#### Scenario: Disable a node
- **WHEN** the user disables a specific node
- **THEN** that node SHALL be excluded from config generation but SHALL remain in the subscription data
