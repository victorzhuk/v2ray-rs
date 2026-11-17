# Spec Delta

## ADDED Requirements

### Requirement: sing-box DNS GeoSite rules register route rule-sets
When generating a sing-box configuration with DNS enabled, every GeoSite category referenced by active custom DNS rules or auto-derived DNS rules SHALL register a corresponding lowercased `geosite-<category>` entry in `route.rule_set` (using local cached `.srs` when present, else remote) and enable `experimental.cache_file` whenever any remote rule-set is emitted.

#### Scenario: Custom DNS GeoSite rule without routing GeoSite rule
- **WHEN** DNS is enabled on sing-box with `use_custom_rules = true`, a DNS rule matches GeoSite `Google` -> `remote`, and routing rules contain no GeoSite rules
- **THEN** `dns.rules` SHALL reference `rule_set: ["geosite-google"]` and `route.rule_set` SHALL contain a matching `geosite-google` definition so `sing-box check` succeeds

#### Scenario: Auto-derived DNS GeoSite rule uses lowercased tag
- **WHEN** DNS is enabled on sing-box with `use_custom_rules = false` and an enabled routing rule matches GeoSite `CN` -> `direct`
- **THEN** both `dns.rules` and `route.rule_set` SHALL use the canonical lowercased tag `geosite-cn`

### Requirement: TUN excluded domains preserve unrestricted DNS servers
When TUN is enabled on v2ray or xray with `exclude_domains` and the selected direct-capable DNS server is not already domain-scoped, the generator SHALL emit a dedicated domain-scoped `skipFallback` entry for the excluded domains while keeping the user's unrestricted server intact for general queries.

#### Scenario: Unrestricted direct server stays available alongside excluded domains
- **WHEN** TUN is enabled on xray with `exclude_domains = ["corp.example.com"]` and DNS has a single unrestricted server `1.1.1.1` with detour `direct`
- **THEN** `dns.servers` SHALL contain both a `domain:corp.example.com`-scoped `skipFallback` entry and the unrestricted `1.1.1.1` server without `skipFallback`, and `Effective DNS` SHALL list both entries accurately

### Requirement: sing-box auto-derived DNS rules preserve domain matcher kind
When DNS is enabled with `use_custom_rules = false` on sing-box, auto-derived DNS rules SHALL preserve each routing rule's matcher kind: `Domain` maps to `domain_suffix`, `DomainKeyword` maps to `domain_keyword`, `DomainFull` maps to `domain`, and `GeoSite` maps to `rule_set`.

#### Scenario: Keyword and full-domain routing rules derive matching sing-box DNS fields
- **WHEN** DNS is enabled on sing-box with auto-derived rules, a direct routing rule has `DomainKeyword("yandex")`, and a proxy routing rule has `DomainFull("api.example.com")`
- **THEN** `dns.rules` SHALL emit `{"domain_keyword": ["yandex"], "server": "domestic"}` and `{"domain": ["api.example.com"], "server": "remote"}` without converting either to `domain_suffix`

### Requirement: Xray probe batches omit unverified TLS nodes
When generating an xray Real Delay probe configuration, TLS nodes with certificate verification disabled (`verify = false`) SHALL be omitted from the probe config while preserving original batch indices on remaining outbounds, matching v2ray's omission of unsupported REALITY and XHTTP nodes.

#### Scenario: Mixed batch with unverified TLS node on xray
- **WHEN** an xray Real Delay batch contains node 0 (`verify = false`) and node 1 (`verify = true`)
- **THEN** the generated probe config SHALL omit node 0, emit node 1 as `probe-1`, and report node 0's delay as unknown rather than failing the whole batch
