## ADDED Requirements

### Requirement: Create routing rules
The system SHALL allow users to create routing rules specifying a match condition and an action.

#### Scenario: Create GeoIP rule
- **WHEN** the user creates a rule with match "GeoIP: RU" and action "direct"
- **THEN** the system SHALL store the rule and include it in config generation

#### Scenario: Create domain pattern rule
- **WHEN** the user creates a rule with match "domain: *.google.com" and action "proxy"
- **THEN** the system SHALL store the rule matching all google.com subdomains

#### Scenario: Create IP CIDR rule
- **WHEN** the user creates a rule with match "IP: 192.168.0.0/16" and action "direct"
- **THEN** the system SHALL store the rule matching the local network range

### Requirement: Edit and delete routing rules
The system SHALL allow users to modify existing rules' match conditions, actions, and to delete rules.

#### Scenario: Change rule action
- **WHEN** the user changes a rule's action from "proxy" to "direct"
- **THEN** the system SHALL update the rule and trigger config regeneration

#### Scenario: Delete rule
- **WHEN** the user deletes a routing rule
- **THEN** the rule SHALL be removed and config SHALL be regenerated

### Requirement: Rule ordering and priority
The system SHALL maintain an ordered list of routing rules, where rules are evaluated from highest to lowest priority. Users SHALL be able to reorder rules.

#### Scenario: Reorder rules
- **WHEN** the user moves a rule from position 5 to position 1
- **THEN** that rule SHALL be evaluated before all others that were previously ahead of it

#### Scenario: First match wins
- **WHEN** traffic matches multiple rules
- **THEN** the first matching rule (highest priority) SHALL determine the action

### Requirement: Rule validation
The system SHALL validate routing rules and provide feedback on errors.

#### Scenario: Invalid country code
- **WHEN** the user enters "XX" as a GeoIP country code
- **THEN** the system SHALL reject the rule with an error message

#### Scenario: Invalid IP CIDR
- **WHEN** the user enters "999.999.999.999/32" as an IP CIDR
- **THEN** the system SHALL reject the rule with an error message

### Requirement: Predefined rule templates
The system SHALL offer predefined routing rule presets that users can apply with one action.

#### Scenario: Apply RU-direct preset
- **WHEN** the user applies the "RU direct" preset
- **THEN** the system SHALL add a GeoIP:RU→direct rule
