# Spec Delta

## ADDED Requirements

### Requirement: URI and VMess parsers support XHTTP and reject unknown transports
Subscription URI and VMess JSON parsers SHALL parse `xhttp` and `splithttp` transports into `TransportSettings::Xhttp` (carrying `path`, `host`, and `mode` defaulting to `auto`) and SHALL return a parse error for unknown transport types instead of falling back to TCP.

#### Scenario: VLESS URI with type=xhttp parses as XHTTP
- **WHEN** `parse_uri` parses a `vless://` or `trojan://` URI with `type=xhttp&path=%2Fsplit&host=cdn.example.com&mode=packet-up`
- **THEN** the resulting node SHALL carry `TransportSettings::Xhttp` with `path = "/split"`, `host = Some("cdn.example.com")`, and `mode = "packet-up"`

#### Scenario: Unknown URI transport fails instead of becoming TCP
- **WHEN** `parse_uri` parses a URI with `type=httpupgrade` or `type=kcp`
- **THEN** `parse_uri` SHALL return `Err(ParseError::InvalidFormat(...))` naming the unsupported transport rather than producing `TransportSettings::Tcp`

### Requirement: IPv6 host normalization across parsers and TCP ping
URI and JSON importers SHALL remove brackets around valid IPv6 literals before storing outbound or DNS addresses. `ProxyNode` validation SHALL reject bracketed addresses. TCP latency probes SHALL dial IPv6 literals with the configured port.

#### Scenario: VLESS IPv6 URI stores bare IPv6 and pings cleanly
- **WHEN** a `vless://uuid@[2001:db8::1]:443?security=tls#v6` URI is parsed and tested with `tcp_ping`
- **THEN** `node.address()` SHALL be `"2001:db8::1"`, `ProxyNode::validate` SHALL accept it while rejecting stray brackets, and `tcp_ping` SHALL dial `[2001:db8::1]:443` without address parse failure

#### Scenario: JSON import normalizes a bracketed IPv6 outbound
- **WHEN** a JSON outbound supplies a valid IPv6 literal enclosed in brackets
- **THEN** the importer SHALL store the bare literal and the model SHALL accept the resulting node
- **AND** direct validation of the same node with its brackets retained SHALL fail

### Requirement: URI scheme case-insensitivity, credential decoding, Trojan TLS, and VMess bounds
URI parsers SHALL accept case-insensitive schemes across all protocols, percent-decode VLESS and Trojan userinfo, default Trojan TLS to enabled on every port unless `security=none` is set, reject oversized VMess base64 inputs before decoding, and validate port numbers within `1..=65535`.

#### Scenario: Uppercase VMess and Shadowsocks schemes parse
- **WHEN** `parse_uri` receives `VMESS://...` or `SS://...` with valid payloads
- **THEN** both URIs SHALL parse identically to lowercase schemes

#### Scenario: Trojan on non-443 port keeps TLS enabled by default
- **WHEN** `parse_uri` receives `trojan://secret%40pass@example.com:8443#node` with no `security` query parameter
- **THEN** the parsed node SHALL have `password = "secret@pass"` and `tls = Some(TlsSettings { verify: true, .. })`

#### Scenario: VMess oversized payload or out-of-range port rejected
- **WHEN** a `vmess://` URI exceeds the encoded length cap for 64 KiB or carries `"port": 70000`
- **THEN** `parse_uri` SHALL fail with `ParseError::InvalidFormat` without allocating an unbounded buffer or truncating the port
