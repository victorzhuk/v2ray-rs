## MODIFIED Requirements

### Requirement: Generate v2ray-compatible configuration
The system SHALL generate a valid JSON configuration file for v2ray/xray containing inbound, outbound, and routing sections.

#### Scenario: Basic SOCKS5 + HTTP inbound with single proxy outbound
- **WHEN** the user has one enabled VLESS node and default settings (SOCKS5 port 1080, HTTP port 1081)
- **THEN** the system SHALL generate a JSON config with SOCKS5 inbound on 1080, HTTP inbound on 1081, a VLESS outbound, and a "freedom" direct outbound

#### Scenario: Multiple proxy nodes with auto-resolve
- **WHEN** the user has multiple enabled nodes and an auto-resolve strategy selected
- **THEN** the system SHALL generate a config for the active connection candidate and refresh it for each candidate attempt

### Requirement: Generate sing-box configuration
The system SHALL generate a valid JSON configuration file in sing-box's configuration schema.

#### Scenario: sing-box basic config
- **WHEN** the user has one enabled Shadowsocks node with sing-box selected
- **THEN** the system SHALL generate a sing-box JSON config with mixed inbound, Shadowsocks outbound, direct outbound, and route rules

### Requirement: Embed routing rules in config
The system SHALL translate the user's routing rules into the backend-specific routing section of the generated config.

#### Scenario: GeoIP direct rule in v2ray config
- **WHEN** the user has a rule "GeoIP:RU → direct"
- **THEN** the v2ray config routing section SHALL contain a rule matching geoip "ru" pointing to the direct outbound tag

#### Scenario: GeoSite proxy rule in sing-box config
- **WHEN** the user has a rule "GeoSite:google → proxy"
- **THEN** the sing-box config route section SHALL contain a rule matching geosite "google" pointing to the proxy outbound tag
