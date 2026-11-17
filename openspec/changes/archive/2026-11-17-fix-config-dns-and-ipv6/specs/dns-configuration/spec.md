# Spec Delta

## MODIFIED Requirements

### Requirement: DNS protocol types
The system SHALL support UDP (plain), TCP, DoH, DoT, DoQ, and H3 DNS protocols, appending custom or default ports as required by each scheme and bracketing bare IPv6 literals whenever a URL authority or `host:port` pair is formatted.

#### Scenario: UDP address formatting
- **WHEN** a DNS server is configured with protocol UDP and address `8.8.8.8`
- **THEN** the system SHALL represent it as `8.8.8.8:53` (default port always appended)

#### Scenario: UDP with custom port
- **WHEN** a DNS server is configured with protocol UDP, address `8.8.8.8`, and port `5353`
- **THEN** the system SHALL represent it as `8.8.8.8:5353`

#### Scenario: TCP address formatting
- **WHEN** a DNS server is configured with protocol TCP and address `8.8.8.8`
- **THEN** the system SHALL produce the address `tcp://8.8.8.8:53`

#### Scenario: DoH address formatting
- **WHEN** a DNS server is configured with protocol DoH and address `1.1.1.1`
- **THEN** the system SHALL produce the address `https://1.1.1.1/dns-query`

#### Scenario: DoT address formatting
- **WHEN** a DNS server is configured with protocol DoT and address `dns.google`
- **THEN** the system SHALL produce the address `tls://dns.google`

#### Scenario: DoQ address formatting
- **WHEN** a DNS server is configured with protocol DoQ and address `dns.adguard.com`
- **THEN** the system SHALL produce the address `quic://dns.adguard.com`

#### Scenario: H3 address formatting
- **WHEN** a DNS server is configured with protocol H3 and address `dns.google`
- **THEN** the system SHALL produce the address `h3://dns.google/dns-query`

#### Scenario: Bare IPv6 literal formatting across UDP and URL schemes
- **WHEN** a DNS server uses bare IPv6 address `2606:4700:4700::1111` with UDP (port `5353`) or DoH (default port)
- **THEN** UDP SHALL format `[2606:4700:4700::1111]:5353` and DoH SHALL format `https://[2606:4700:4700::1111]/dns-query`

### Requirement: DNS validation covers FakeIP CIDR ranges
The DNS model SHALL validate FakeIP CIDR ranges when FakeIP is enabled, requiring `inet4_range` to be a valid IPv4 CIDR and `inet6_range` to be a valid IPv6 CIDR.

#### Scenario: Invalid FakeIP IPv4 range
- **WHEN** FakeIP is enabled and the user configures an invalid FakeIP IPv4 CIDR range or an IPv6 CIDR such as `fc00::/18` in `inet4_range`
- **THEN** the DNS model SHALL reject the configuration with a validation error

#### Scenario: Invalid FakeIP IPv6 range
- **WHEN** FakeIP is enabled and the user configures an invalid FakeIP IPv6 CIDR range or an IPv4 CIDR such as `198.18.0.0/15` in `inet6_range`
- **THEN** the DNS model SHALL reject the configuration with a validation error

#### Scenario: FakeIP validation skipped when disabled
- **WHEN** FakeIP is disabled
- **THEN** the DNS model SHALL NOT validate the CIDR range values, allowing them to hold any string without error until FakeIP is enabled

## ADDED Requirements

### Requirement: DNS validation covers reserved tags, rule values, host overrides, and TUN path
`DnsConfig` validation SHALL reject reserved internal server tags (`hosts`, `fakeip`, `sys-dns-bootstrap`, `tun-dns-fallback`), validate every `DnsRuleMatch` and `HostOverride` domain and IP, and validate `client_subnet` and `hosts` whenever TUN is enabled even if `dns.enabled` is `false`.

#### Scenario: Reserved server tag rejected
- **WHEN** a DNS server is configured with tag `hosts`, `fakeip`, `sys-dns-bootstrap`, or `tun-dns-fallback`
- **THEN** validation SHALL reject the server with an error naming the reserved tag

#### Scenario: Invalid host override or client subnet rejected on TUN derived DNS path
- **WHEN** `dns.enabled` is `false`, `tun.enabled` is `true`, and `dns.client_subnet` or `dns.hosts` contains an invalid IP or domain
- **THEN** runtime validation SHALL fail before config generation with an error naming the invalid DNS field
