## ADDED Requirements

### Requirement: Effective DNS summary
The DNS page SHALL show a read-only "Effective DNS" summary of the resolvers a connection with the current settings, selected backend, and TUN state would use, listing each resolver's address, transport, path, source, and scope. The summary SHALL stay visible and readable while the DNS master toggle is off. It SHALL update when DNS, TUN, or backend settings change. When one or more subscriptions have an enabled imported profile with DNS settings, the summary SHALL name those subscriptions and state that their nodes use the profile's DNS instead.

#### Scenario: Fallback shown while DNS is off
- **WHEN** the backend is xray, TUN is enabled, DNS is disabled, and the user opens the DNS page
- **THEN** the summary SHALL list `https://1.1.1.1/dns-query` and `https://8.8.8.8/dns-query` as fallback resolvers reached through the proxy

#### Scenario: Summary follows edits
- **WHEN** the user enables DNS on that page
- **THEN** the summary SHALL replace the fallback entries with the configured servers without reopening Preferences

#### Scenario: Imported profile named
- **WHEN** a subscription named "Provider" has an enabled imported profile with DNS settings
- **THEN** the summary SHALL state that nodes from "Provider" use that profile's DNS
