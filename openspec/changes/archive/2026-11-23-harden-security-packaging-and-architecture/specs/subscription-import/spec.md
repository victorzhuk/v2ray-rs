# Spec Delta: subscription-import

## ADDED Requirements

### Requirement: Trojan URI parsing enforces TLS security checks when security is omitted
`parse_trojan` SHALL route omitted or empty `security` parameters through `parse_url_tls` so `allowInsecure` is rejected and `sni`, `alpn`, and `fp` are preserved, defaulting `server_name` to the server address only when `sni` is absent. Unknown `security` values SHALL be rejected.

#### Scenario: Omitted security preserves SNI and rejects allowInsecure
- **WHEN** `parse_trojan` parses `trojan://pw@example.com:443?sni=cdn.example.com&fp=chrome`
- **THEN** the parsed node SHALL preserve `server_name = "cdn.example.com"` and `fingerprint = "chrome"`
- **AND** adding `allowInsecure=1` SHALL return a parse error

#### Scenario: Unknown security parameter is rejected
- **WHEN** `parse_trojan` parses `trojan://pw@example.com:443?security=bogus`
- **THEN** parsing SHALL return an error instead of silently falling back

### Requirement: Subscription fetch SSRF guard blocks 0.0.0.0/8, multicast, and reserved ranges
`ip_is_private_or_local` SHALL reject IPv4 `0.0.0.0/8`, multicast (`224.0.0.0/4`), reserved (`240.0.0.0/4`), IPv6 multicast (`ff00::/8`), and NAT64-embedded private IPv4 addresses (`64:ff9b::/96`).

#### Scenario: IPv4 0.0.0.0/8 target is rejected as local
- **WHEN** a subscription URL or redirect resolves to `0.0.0.1` or `0.1.2.3`
- **THEN** the SSRF guard SHALL treat it as local and refuse the request
