## ADDED Requirements

### Requirement: Built-in DNS provider presets
The system SHALL provide a hardcoded list of DNS provider presets. Each preset SHALL contain a name, description, a list of DNS server configurations, and a DNS strategy.

#### Scenario: Built-in presets available
- **WHEN** the system initializes
- **THEN** at least 8 built-in DNS provider presets SHALL be available: Cloudflare, Cloudflare Family, Google, AdGuard, AdGuard Family, Quad9, Ali DNS, Yandex DNS

#### Scenario: Each preset provides valid server configs
- **WHEN** a preset is accessed
- **THEN** it SHALL contain at least 2 DNS server configs with unique tags ("remote" and "domestic"), valid protocols, and valid addresses

### Requirement: Apply DNS provider preset
Applying a DNS provider preset SHALL replace the current DNS server list and strategy. It SHALL enable DNS if not already enabled. It SHALL NOT modify DNS rules, FakeIP, cache, client subnet, or host overrides.

#### Scenario: Apply replaces servers and strategy
- **WHEN** the user applies the "Cloudflare" preset
- **THEN** `dns.servers` SHALL be replaced with Cloudflare's servers (DoH 1.1.1.1 remote, UDP 1.0.0.1 domestic), `dns.strategy` SHALL be set to PreferIpv4, and `dns.enabled` SHALL be true

#### Scenario: Apply preserves other DNS settings
- **WHEN** the user has custom DNS rules, FakeIP enabled, and host overrides, then applies a preset
- **THEN** FakeIP config and host overrides SHALL remain unchanged. DNS rules whose `server_tag` references a server still present in the new list SHALL be preserved; rules referencing removed server tags SHALL be dropped.

#### Scenario: Apply overwrites previous preset
- **WHEN** the user applies "Google" preset after previously applying "Cloudflare"
- **THEN** the server list SHALL contain only Google's servers, not a mix of both

### Requirement: Preset server conventions
Each preset's remote server SHALL use DoH protocol (encrypted, suitable for proxy routing) and the domestic server SHALL use UDP protocol (fast, suitable for direct routing).

#### Scenario: Cloudflare preset servers
- **WHEN** the Cloudflare preset is accessed
- **THEN** it SHALL have a remote server with DoH protocol and address "1.1.1.1", and a domestic server with UDP protocol and address "1.0.0.1"

#### Scenario: AdGuard preset servers
- **WHEN** the AdGuard preset is accessed
- **THEN** it SHALL have a remote server with DoH protocol and address "dns.adguard.com", and a domestic server with UDP protocol and address "94.140.14.14"

#### Scenario: Yandex preset servers
- **WHEN** the Yandex DNS preset is accessed
- **THEN** it SHALL have a remote server with DoH protocol and address "common.dot.dns.yandex.net", and a domestic server with UDP protocol and address "77.88.8.8"
