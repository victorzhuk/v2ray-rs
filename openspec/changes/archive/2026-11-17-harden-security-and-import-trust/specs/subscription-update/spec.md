# Spec Delta

## ADDED Requirements

### Requirement: Subscription HTTP redirects block private and local targets
Subscription HTTP fetches SHALL evaluate each redirect hop and reject redirects from public origins to loopback, link-local, RFC 1918 private, unique-local, unspecified IP literals, or `localhost` hostnames.

#### Scenario: Public subscription redirects to loopback or metadata IP
- **WHEN** fetching an `https://` subscription URL returns a `302` redirect to `http://127.0.0.1:8080/sub` or `http://169.254.169.254/latest/meta-data/`
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
