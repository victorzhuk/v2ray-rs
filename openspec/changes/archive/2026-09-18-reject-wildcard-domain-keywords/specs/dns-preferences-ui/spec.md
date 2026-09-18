## ADDED Requirements

### Requirement: DNS rule keyword values are validated
The DNS rule dialog SHALL validate a Domain Keyword value with the same rule as routing keyword rules and SHALL NOT save a value containing `*` or whitespace, showing the validation message inline. Stored DNS keyword rules with such a value SHALL stay unchanged. Their rows SHALL be marked invalid with error styling and use the validation message as their subtitle.

#### Scenario: Wildcard DNS keyword rejected
- **WHEN** the user adds a DNS rule with match type Domain Keyword and value `*.cn`
- **THEN** the dialog SHALL show an inline error and the rule SHALL NOT be saved

#### Scenario: Stored wildcard DNS keyword surfaced
- **WHEN** stored settings contain a DNS keyword rule `*.cn`
- **THEN** the settings SHALL load unchanged and the rule's row SHALL show the validation error as its subtitle
