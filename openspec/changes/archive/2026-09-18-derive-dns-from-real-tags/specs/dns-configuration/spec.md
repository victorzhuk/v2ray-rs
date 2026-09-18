## ADDED Requirements

### Requirement: Auto-derived DNS rules require their server tags
When DNS is enabled and `use_custom_rules` is false, the generators SHALL emit derived DNS rules for proxy-action domains only when a server tagged `remote` exists, and for direct-action domains only when a server tagged `domestic` exists. For each missing tag that derived rules would have used, generation SHALL log a warning naming the tag and the number of skipped domain entries, and SHALL NOT emit any DNS rule or `domains` entry referencing a server that is not in the generated config.

#### Scenario: Missing domestic tag on sing-box
- **WHEN** the backend is sing-box, auto-derived rules are active, servers are tagged `remote` and `lan`, and a direct routing rule has GeoSite `category-ru`
- **THEN** `dns.rules` SHALL contain no rule with `"server": "domestic"`, a warning naming `domestic` SHALL be logged, and the config SHALL pass `sing-box check`

#### Scenario: Missing remote tag on xray
- **WHEN** the backend is xray, auto-derived rules are active, no server is tagged `remote`, and a proxy routing rule has domain pattern `example.com`
- **THEN** no DNS server SHALL carry `domain:example.com` in `domains` and a warning naming `remote` SHALL be logged

#### Scenario: Standard tags unaffected
- **WHEN** servers are tagged `remote` and `domestic`
- **THEN** derived DNS rules SHALL be emitted as before and no warning SHALL be logged