## MODIFIED Requirements

### Requirement: Embed routing rules in config
The system SHALL translate the user's routing rules into the backend-specific routing section of the generated config. For sing-box, GeoIP/GeoSite rules SHALL reference `type: remote` rule-sets without a `download_detour` field, so rule-set downloads go through sing-box's default outbound (the proxy), and any config referencing at least one remote rule-set SHALL enable `experimental.cache_file` with an absolute path under the profile's cache directory so fetched rule-sets persist across restarts.

#### Scenario: GeoIP direct rule in v2ray config
- **WHEN** the user has a rule "GeoIP:RU → direct"
- **THEN** the v2ray config routing section SHALL contain a rule matching geoip "ru" pointing to the direct outbound tag

#### Scenario: GeoSite proxy rule in sing-box config
- **WHEN** the user has a rule "GeoSite:google → proxy"
- **THEN** the sing-box config route section SHALL contain a rule matching geosite "google" pointing to the proxy outbound tag

#### Scenario: sing-box remote rule-sets download via the default outbound
- **WHEN** the user has any GeoIP or GeoSite rule and sing-box is the selected backend
- **THEN** each emitted `route.rule_set` entry SHALL have `type: "remote"` and SHALL NOT contain a `download_detour` field

#### Scenario: sing-box rule-set cache enabled
- **WHEN** the generated sing-box config references at least one remote rule-set
- **THEN** the config SHALL contain `experimental.cache_file` with `"enabled": true` and an absolute `path` under the profile's cache directory

#### Scenario: FakeIP mappings persisted when cache file present
- **WHEN** the generated sing-box config enables both the cache file and FakeIP
- **THEN** `experimental.cache_file` SHALL include `"store_fakeip": true`

#### Scenario: No cache file without remote rule-sets
- **WHEN** the routing rules reference no GeoIP or GeoSite rule-sets and FakeIP is disabled
- **THEN** the sing-box config SHALL NOT emit an `experimental.cache_file` section
