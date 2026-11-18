## Purpose

Defines how subscriptions are fetched, retried, and reconciled — including fetch-error classification (transient vs terminal), automatic periodic updates, and node reconciliation that preserves user preferences.

## Requirements

### Requirement: Manual subscription update
The system SHALL surface partial parse failures without discarding valid nodes.

#### Scenario: Manual update with mixed valid and invalid URIs
- **WHEN** an update source returns valid proxy URIs together with invalid entries
- **THEN** the system SHALL keep the valid nodes, report the skipped entries, and persist the reconciled result

### Requirement: Automatic subscription update
The system SHALL support automatic periodic updates of subscriptions based on a configurable interval.

#### Scenario: Auto-update triggers
- **WHEN** the configured auto-update interval elapses
- **THEN** the system SHALL fetch and update all subscriptions with auto-update enabled

#### Scenario: Auto-update failure
- **WHEN** an auto-update fails due to a transient error (HTTP 408, 429, any 5xx, connection failure, or timeout)
- **THEN** the system SHALL retry up to 3 times with exponential backoff and log the failure

#### Scenario: Terminal fetch error fails fast
- **WHEN** a subscription fetch fails with a terminal error (any other 4xx status, a malformed or non-http(s) URL, or a request that cannot be constructed)
- **THEN** the system SHALL fail immediately without retrying or sleeping and SHALL report an error that identifies the terminal cause

### Requirement: Update node reconciliation
The system SHALL reconcile updated subscription nodes with existing data, matching nodes by address, port, and protocol to preserve user preferences.

#### Scenario: Matching node keeps enabled state
- **WHEN** an updated subscription still contains a previously known node
- **THEN** the system SHALL preserve that node's enabled state

### Requirement: Fetch error classification
The subscription fetcher SHALL distinguish malformed or unsupported URLs from network-level failures in its error type, so callers and error messages do not present a permanently invalid source as a transient network problem.

#### Scenario: Malformed URL reported distinctly
- **WHEN** a subscription source URL is not a valid http(s) URL
- **THEN** the fetch error SHALL identify it as an invalid URL rather than a network error

### Requirement: Subscription refresh is offered on REALITY auth rejection
When a connect candidate's failure is classified as REALITY auth rejection and the candidate's node belongs to a subscription, the failure report SHALL offer a refresh action for that subscription. The offer SHALL appear at most once per connection per subscription regardless of how many of the subscription's nodes were classified, and the refresh itself SHALL remain user-initiated through the existing manual update flow.

#### Scenario: Offer names the owning subscription
- **WHEN** a REALITY auth rejection is classified on a node from subscription S
- **THEN** the failure report SHALL offer a refresh action for S

#### Scenario: One offer per connection per subscription
- **WHEN** candidates from subscription S are classified twice within one connection
- **THEN** the refresh offer SHALL appear exactly once

#### Scenario: Refresh stays user-initiated
- **WHEN** the classification fires
- **THEN** the system SHALL NOT start a subscription update on its own; the update SHALL run only after the user activates the offered action

#### Scenario: Classification on a manual node offers nothing
- **WHEN** a REALITY auth rejection is classified on a manual node with no owning subscription
- **THEN** the failure report SHALL show the classification without a refresh offer

### Requirement: Safe subscription imports
The system SHALL exclude arbitrary HTTP error bodies from routine errors and SHALL bound regular local-file imports to 10 MiB without blocking asynchronous workers.
#### Scenario: Oversized local source
- **WHEN** a local source exceeds 10 MiB or is not a regular file
- **THEN** the import SHALL fail without replacing existing subscription data
#### Scenario: Server rejects a refresh
- **WHEN** the subscription server returns an HTTP error with a response body
- **THEN** the reported fetch error SHALL omit the body and the existing subscription SHALL remain intact

### Requirement: Connection identity reconciliation
The system SHALL match unchanged connection settings before endpoint fallbacks, ignoring remarks. Endpoint fallback SHALL apply only to unique unmatched pairs. Changed connection settings SHALL invalidate latency samples.
#### Scenario: Reordered endpoint duplicates
- **WHEN** distinct connection configurations sharing an endpoint are reordered
- **THEN** each configuration SHALL retain its own identity and enabled state

#### Scenario: A changed connection has an outstanding measurement
- **WHEN** unique endpoint fallback retains a node identity after its connection settings change
- **THEN** the system SHALL invalidate saved latency samples for the old settings
- **AND** a probe started with the old settings SHALL NOT restore a stale sample when it finishes

### Requirement: Subscription HTTP redirects block private and local targets
Subscription HTTP fetches SHALL evaluate each redirect hop and reject redirects to loopback, link-local, RFC 1918 private, RFC 6598 shared address space (`100.64.0.0/10`), RFC 2544 benchmarking (`198.18.0.0/15`), unique-local, unspecified, or broadcast IP literals (including IPv4-mapped IPv6 forms), or `localhost` hostnames.

#### Scenario: Public subscription redirects to loopback or metadata IP
- **WHEN** fetching an `https://` subscription URL returns a `302` redirect to `http://127.0.0.1:8080/sub` or `http://169.254.169.254/latest/meta-data/`
- **THEN** the fetcher SHALL abort the redirect and return a terminal fetch error without connecting to the redirect target

#### Scenario: Public subscription redirects to CGNAT or benchmarking IP
- **WHEN** fetching an `https://` subscription URL returns a redirect to `https://100.64.0.1/sub`, `https://198.18.0.1/sub`, or `https://[::ffff:100.64.0.1]/sub`
- **THEN** the fetcher SHALL abort the redirect and return a terminal fetch error without connecting to the redirect target

#### Scenario: Public-to-public HTTPS redirect succeeds
- **WHEN** fetching a subscription URL redirects across public hosts within the redirect limit
- **THEN** the fetcher SHALL follow the redirect normally

### Requirement: Remote subscription profiles require explicit opt-in
Subscriptions created from HTTP or HTTPS URLs SHALL default `use_imported_profile` to `false`, and missing persisted values SHALL deserialize as `false` while preserving explicit booleans. Local-file and pasted JSON bundle imports SHALL default `use_imported_profile` to `true`, and the UI SHALL expose a toggle per subscription carrying an imported profile.

#### Scenario: URL subscription keeps imported profile inactive until enabled
- **WHEN** a user adds an `https://` subscription returning a full Xray JSON config with DNS and routing sections
- **THEN** the subscription's nodes SHALL be imported with `use_imported_profile = false`, its DNS and routing profile SHALL NOT override user settings until the user enables the profile toggle in the subscription UI, and a notice SHALL state that an imported profile is available

#### Scenario: Local JSON file import enables profile by default
- **WHEN** a user imports a full Xray JSON bundle from a local file or pasted JSON text
- **THEN** `use_imported_profile` SHALL default to `true`

#### Scenario: Existing persisted boolean survives reload
- **WHEN** `subscriptions.json` contains an explicit `"use_imported_profile": true` or `"use_imported_profile": false`, or omits the field on legacy records
- **THEN** explicit booleans SHALL load unchanged and omitted fields SHALL default to `false`

### Requirement: Imported TLS nodes with disabled verification are rejected
Subscription URI and JSON importers SHALL reject TLS proxy nodes that disable certificate verification (`allowInsecure`) and SHALL report a warning naming the skipped node without aborting valid nodes in the same source.

#### Scenario: JSON outbound with allowInsecure is skipped with warning
- **WHEN** an imported JSON bundle contains a TLS outbound with `"allowInsecure": true` alongside a valid TLS outbound
- **THEN** the `allowInsecure` outbound SHALL be omitted from imported nodes, the valid outbound SHALL be kept, and a warning naming the skipped outbound SHALL be returned

#### Scenario: URI with allowInsecure=1 is skipped with warning
- **WHEN** a subscription line contains `allowInsecure=1` or `allowInsecure=true` on a TLS URI
- **THEN** parsing that URI SHALL fail with an error naming disabled TLS verification and other valid URIs in the subscription SHALL still import

### Requirement: Imported Xray routing rules preserve single-condition semantics
When importing Xray JSON routing rules, the importer SHALL ignore `protocol: "dns"` outbounds instead of mapping them to direct, fold `network: "tcp,udp"` as a no-op when paired with a single supported matcher, and drop any rule carrying multiple matcher axes or unsupported matchers as a whole with a surfaced warning.

#### Scenario: Single matcher with tcp,udp imports cleanly
- **WHEN** an Xray JSON routing rule specifies `"ip": ["geoip:private"]`, `"network": "tcp,udp"`, and `"outboundTag": "direct"`
- **THEN** the importer SHALL import the rule as `GeoIP(PRIVATE) -> Direct` without warning

#### Scenario: Multi-axis QUIC block rule is dropped whole instead of widened
- **WHEN** an Xray JSON routing rule specifies `"network": "udp"`, `"port": "443"`, and `"outboundTag": "block"`
- **THEN** the importer SHALL drop the entire rule and emit a warning instead of importing an unconditional UDP block rule

#### Scenario: DNS outbound rule is not mapped to direct
- **WHEN** an Xray JSON bundle defines `{"tag": "dns-out", "protocol": "dns"}` and a routing rule targeting `"dns-out"`
- **THEN** `"dns-out"` SHALL NOT be classified as a direct outbound and rules targeting it SHALL be skipped cleanly

### Requirement: Imported profile DNS and routing inputs are validated
Imported profile DNS servers, DNS rules, static host overrides, and routing rules SHALL be validated through the core domain validators both during JSON import and before runtime config generation, rejecting invalid entries instead of emitting unchecked backend JSON.

#### Scenario: Invalid imported routing rule or DNS server is rejected
- **WHEN** an imported JSON profile contains a malformed CIDR, invalid domain pattern, invalid DNS server address, or reserved server tag
- **THEN** the importer SHALL drop or reject the invalid profile element with a warning, and runtime config generation SHALL validate active imported profiles with the same rules as user settings

#### Scenario: Imported wildcard full-domain rule is skipped with warning
- **WHEN** an imported Xray JSON profile contains `"full:*.example.com"` in a routing rule or DNS server `domains` list
- **THEN** the importer SHALL skip that matcher entry and record a warning without aborting valid rules

#### Scenario: Imported wildcard or TLD DNS suffix rule is accepted
- **WHEN** an imported Xray JSON profile contains `"domain:*.example.com"` or `"domain:cn"` in a DNS server `domains` list
- **THEN** the importer SHALL keep those DNS suffix rules and runtime validation SHALL accept the imported DNS config

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

### Requirement: Xray JSON DNS server schemes and pseudo-servers import accurately
When importing an Xray JSON `dns` section, `https+local://`, `quic+local://`, and `tcp+local://` server URLs SHALL map to `Doh`, `Doq`, and `Tcp` protocols with `detour = "direct"`. Built-in `localhost` and `fakedns` pseudo-servers and DoH/H3 URLs carrying non-standard paths other than `/dns-query` SHALL be skipped with a surfaced warning rather than coerced into plain UDP port `53` servers.

#### Scenario: Local-mode Xray DNS URLs preserve transport and direct detour
- **WHEN** an Xray JSON bundle contains `"https+local://1.1.1.1/dns-query"` and `"quic+local://dns.adguard.com:853"`
- **THEN** the importer SHALL produce a `Doh` server (`1.1.1.1`, detour `"direct"`) and a `Doq` server (`dns.adguard.com`, port `853`, detour `"direct"`)

#### Scenario: Pseudo-servers and custom DoH paths are skipped with warnings
- **WHEN** an Xray JSON bundle contains `"localhost"`, `"fakedns"`, or `"https://dns.example.com/custom-endpoint"`
- **THEN** the importer SHALL skip those DNS entries, emit a warning naming each skipped server, and SHALL NOT emit a `Udp` server on port `53` for them

### Requirement: Subscription HTTP fetches block private initial URLs and DNS answers
Subscription HTTP fetches SHALL validate the initial request URL and every resolved DNS socket address against the same public-target policy applied to redirects, rejecting loopback, link-local, RFC 1918 private, RFC 6598 shared, RFC 2544 benchmarking, unique-local, unspecified, broadcast, IPv4-mapped private IPv6 addresses, and `localhost` hostnames before connecting.

#### Scenario: Direct private or metadata subscription URL is rejected before connect
- **WHEN** a user or `v2raytun://import/` link supplies `http://127.0.0.1:8080/sub`, `http://169.254.169.254/latest/meta-data/`, or `https://localhost/sub`
- **THEN** the subscription fetch SHALL fail immediately as a non-transient error without opening a socket or retrying

#### Scenario: Public hostname resolving to a private IP is rejected
- **WHEN** a subscription URL uses a non-`localhost` domain name whose DNS lookup returns `127.0.0.1`, `10.0.0.1`, `169.254.169.254`, or `::1`
- **THEN** the subscription client SHALL refuse the resolved address and fail the fetch before establishing a TCP or TLS connection

### Requirement: Subscription imports bound parsed node count
Subscription imports and refreshes across URI-list and JSON configuration-bundle sources SHALL enforce an upper bound of 4,096 parsed proxy nodes per subscription, failing oversized node lists without replacing existing subscription data or rendering unbounded widget trees.

#### Scenario: URI subscription exceeding maximum node count fails cleanly
- **WHEN** a fetched or local subscription decodes to more than 4,096 valid proxy nodes within the 10 MiB payload limit
- **THEN** the import SHALL return a descriptive error naming the node limit without persisting the oversized subscription

#### Scenario: Refresh exceeding node cap preserves previous subscription nodes
- **WHEN** refreshing an existing subscription produces more than 4,096 parsed nodes
- **THEN** the refresh SHALL fail and the previously stored nodes for that subscription SHALL remain intact
