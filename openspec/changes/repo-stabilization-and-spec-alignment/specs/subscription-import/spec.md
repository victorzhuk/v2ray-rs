## MODIFIED Requirements

### Requirement: Import subscription from URL
The system SHALL report partial parse failures while still importing valid nodes, and it SHALL avoid persisting a new subscription when no valid nodes are produced.

#### Scenario: Partial success import
- **WHEN** the source contains valid proxy URIs and invalid URIs together
- **THEN** the system SHALL import the valid nodes, persist the subscription, and surface which entries were skipped

#### Scenario: No valid nodes
- **WHEN** a new import source yields zero valid proxy nodes
- **THEN** the system SHALL report the failure and SHALL NOT persist an empty subscription

### Requirement: Import subscription from file
The system SHALL allow file-based imports from onboarding and the main subscriptions page.

#### Scenario: File path entered during onboarding
- **WHEN** the user provides a local subscription file path during onboarding
- **THEN** the onboarding flow SHALL create a file-backed subscription source instead of requiring a URL

