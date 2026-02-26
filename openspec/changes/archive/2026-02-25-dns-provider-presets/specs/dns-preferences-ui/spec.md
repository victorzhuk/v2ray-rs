## MODIFIED Requirements

### Requirement: DNS server list management
The DNS page SHALL display the list of configured DNS servers with controls to add, edit, and remove servers. The Servers group SHALL include a "Providers" button that opens a provider picker dialog.

#### Scenario: Add DNS server via dialog
- **WHEN** the user clicks the add button in the Servers group
- **THEN** a dialog SHALL appear with fields for tag, protocol (dropdown), address, port, and detour (optional dropdown, visible only when backend is sing-box)

#### Scenario: Edit existing DNS server
- **WHEN** the user activates an existing server row
- **THEN** an edit dialog SHALL appear pre-filled with the server's current settings

#### Scenario: Remove DNS server
- **WHEN** the user removes a DNS server that is referenced by a DNS rule
- **THEN** the system SHALL warn the user before removing and clean up referencing rules

#### Scenario: Default servers shown on first open
- **WHEN** the user opens DNS Preferences for the first time
- **THEN** the server list SHALL show the two default servers (remote DoH 1.1.1.1, domestic UDP 223.5.5.5)

#### Scenario: Providers button visible in Servers group
- **WHEN** the DNS page is displayed with DNS enabled
- **THEN** the Servers group SHALL include a "Providers" button

#### Scenario: Provider picker dialog
- **WHEN** the user clicks the "Providers" button
- **THEN** a dialog SHALL appear listing all built-in DNS providers, each with name, description, and an "Apply" button

#### Scenario: Apply shows confirmation before replacing
- **WHEN** the user clicks "Apply" on a provider in the dialog
- **THEN** a confirmation prompt SHALL appear asking to confirm replacing current DNS servers

#### Scenario: Apply confirmed updates servers
- **WHEN** the user confirms the apply action
- **THEN** the DNS server list in preferences SHALL update immediately to show the provider's servers, and the strategy dropdown SHALL update to the provider's strategy

#### Scenario: Apply cancelled preserves servers
- **WHEN** the user cancels the apply confirmation
- **THEN** the current DNS server list SHALL remain unchanged

#### Scenario: Apply provider persists settings
- **WHEN** the user confirms applying a provider preset
- **THEN** the settings SHALL be saved via the existing auto-persist callback
