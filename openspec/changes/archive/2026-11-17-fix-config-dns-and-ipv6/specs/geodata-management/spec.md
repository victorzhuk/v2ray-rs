# Spec Delta

## ADDED Requirements

### Requirement: sing-box rule-set collection includes active DNS GeoSite tags
For sing-box, the set of required rule-set tags SHALL include lowercased `geosite-<category>` tags from active custom DNS rules (when `use_custom_rules` is `true`) or auto-derived DNS rules (when `use_custom_rules` is `false`) whenever DNS is enabled, alongside routing rule tags.

#### Scenario: Custom DNS GeoSite tag is prefetched on refresh
- **WHEN** sing-box is selected, DNS is enabled with custom rule `GeoSite("openai") -> "remote"`, and a geodata refresh runs
- **THEN** `geosite-openai.srs` SHALL be included in the downloaded rule-set tags even if no routing rule references `openai`

### Requirement: Scheduled and manual refreshes update existing sing-box rule-sets
Scheduled background refreshes and manual "Update Now" actions for sing-box SHALL re-download all required `.srs` rule-set files rather than skipping tags that already exist on disk, while startup hydration MAY fetch only missing tags.

#### Scenario: Manual Update Now refreshes cached rule-sets
- **WHEN** `geosite-google.srs` already exists in `cache_dir/geodata/rule-sets/` and the user clicks "Update Now" (or the scheduled update interval fires)
- **THEN** the refresh pass SHALL fetch the latest `geosite-google.srs` atomically and update the last-refresh timestamp
