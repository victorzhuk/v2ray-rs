## MODIFIED Requirements

### Requirement: Apply DNS provider preset
Applying a DNS provider preset SHALL replace the current DNS server list and strategy. It SHALL enable DNS if not already enabled. It SHALL NOT introduce any merge behavior.

#### Scenario: Apply confirmed replaces servers
- **WHEN** the user confirms applying a preset over a customized DNS server list
- **THEN** `dns.servers` is replaced with only the preset servers and `dns.strategy` is set to the preset strategy

#### Scenario: Apply cancelled preserves current DNS config
- **WHEN** the user cancels the preset confirmation dialog
- **THEN** the existing DNS server list and strategy remain unchanged
