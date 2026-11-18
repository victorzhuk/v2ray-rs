# Spec Delta

## ADDED Requirements

### Requirement: Full-domain rules reject wildcards
The system SHALL reject a full-domain routing rule whose value contains `*`, with an error stating that full-domain rules match an exact host and pointing users to suffix domain rules for wildcards. Rules already stored with `*` SHALL load without error and SHALL NOT be rewritten automatically. The routing rule list SHALL mark each stored invalid rule with error styling and show the validation message as its subtitle until the user edits or deletes it.

#### Scenario: Wildcard full-domain rule rejected on save
- **WHEN** the user creates or edits a routing rule with match `Domain Full: *.example.com`
- **THEN** the system SHALL reject the rule with a wildcard error and SHALL NOT persist it

#### Scenario: Stored wildcard full-domain rule surfaced on load
- **WHEN** stored routing rules contain a full-domain rule `*.example.com`
- **THEN** the rules SHALL load with `*.example.com` unchanged and its row SHALL show the validation error as its subtitle

#### Scenario: Exact hostname accepted in full-domain rule
- **WHEN** the user creates a routing rule with match `Domain Full: api.example.com`
- **THEN** the system SHALL store the rule
