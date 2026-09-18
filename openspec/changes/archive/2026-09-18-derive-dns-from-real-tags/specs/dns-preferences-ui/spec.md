## ADDED Requirements

### Requirement: Missing auto-split tag is shown
While custom DNS rules are off, the primary `remote` or `domestic` row whose tag has no configured server SHALL state that DNS rules derived from routing for that server are skipped, instead of only "Not configured".

#### Scenario: Domestic tag missing in auto mode
- **WHEN** `use_custom_rules` is false and no server is tagged `domestic`
- **THEN** the Domestic row SHALL state that routing-derived DNS rules for direct traffic are skipped

#### Scenario: Custom rules active
- **WHEN** `use_custom_rules` is true and no server is tagged `domestic`
- **THEN** the Domestic row SHALL show "Not configured" without the skipped-rules note