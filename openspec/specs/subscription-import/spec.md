## ADDED Requirements

### Requirement: Import subscription from URL
The system SHALL fetch subscription content from a user-provided URL via HTTP(S), decode it, and parse it into proxy nodes.

#### Scenario: Base64-encoded subscription
- **WHEN** the user provides a URL that returns a base64-encoded list of proxy URIs (one per line after decoding)
- **THEN** the system SHALL decode the response, split by newline, and parse each URI into a proxy node

#### Scenario: Network error
- **WHEN** the subscription URL is unreachable or returns a non-200 status
- **THEN** the system SHALL report a clear error to the user without crashing

#### Scenario: Invalid content
- **WHEN** the subscription URL returns content that cannot be parsed as proxy URIs
- **THEN** the system SHALL report which URIs failed parsing and still import any valid ones

### Requirement: Import subscription from file
The system SHALL import subscription data from a local file in the same formats as URL-based import.

#### Scenario: Valid file import
- **WHEN** the user selects a local file containing base64-encoded proxy URIs
- **THEN** the system SHALL parse it identically to a URL response

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
