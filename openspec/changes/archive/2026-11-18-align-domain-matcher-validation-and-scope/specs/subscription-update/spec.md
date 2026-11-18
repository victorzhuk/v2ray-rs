# Spec Delta

## MODIFIED Requirements

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
