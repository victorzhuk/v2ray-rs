## ADDED Requirements

### Requirement: Keyword rules reject wildcards
The system SHALL reject a domain keyword rule whose value contains `*`, with an error stating that a keyword is a plain substring and pointing users to the Domain rule type for wildcard suffixes. Rules already stored with such a keyword SHALL load without error and SHALL NOT be rewritten automatically. The routing rule list SHALL mark each stored invalid rule with error styling and use the validation message as its subtitle until the user edits or deletes it.

#### Scenario: Wildcard keyword rejected on save
- **WHEN** the user creates or edits a rule with match `Domain Keyword: *.ru`
- **THEN** the system SHALL reject the rule and SHALL NOT persist it

#### Scenario: Stored wildcard keyword surfaced
- **WHEN** stored routing rules contain a keyword rule `*.ru` from an earlier version
- **THEN** the rules SHALL load, the stored value SHALL stay `*.ru`, and its row SHALL show the validation error as its subtitle

#### Scenario: Plain keyword accepted
- **WHEN** the user creates a rule with match `Domain Keyword: sina`
- **THEN** the system SHALL store the rule
