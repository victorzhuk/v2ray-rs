## ADDED Requirements

### Requirement: DNS validation covers FakeIP CIDR ranges
The DNS model SHALL validate FakeIP IPv4 and IPv6 CIDR ranges when FakeIP is enabled. Duplicate server tags, invalid rule targets, and invalid client subnet values were already validated before this change and are codified here for reference, not newly added.

#### Scenario: Invalid FakeIP IPv4 range
- **WHEN** FakeIP is enabled and the user configures an invalid FakeIP IPv4 CIDR range
- **THEN** the DNS model rejects the configuration with a validation error

#### Scenario: Invalid FakeIP IPv6 range
- **WHEN** FakeIP is enabled and the user configures an invalid FakeIP IPv6 CIDR range
- **THEN** the DNS model rejects the configuration with a validation error

#### Scenario: FakeIP validation skipped when disabled
- **WHEN** FakeIP is disabled
- **THEN** the DNS model SHALL NOT validate the CIDR range values, allowing them to hold any string without error until FakeIP is enabled

#### Scenario: DNS rule references missing server
- **WHEN** a DNS rule targets a server tag that is not present in the server list
- **THEN** the DNS model rejects the configuration with a validation error
