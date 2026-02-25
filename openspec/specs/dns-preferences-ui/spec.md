## ADDED Requirements

### Requirement: DNS Preferences page
The Preferences dialog SHALL include a "DNS" page with icon `network-transmit-symbolic` that provides controls for all DNS configuration settings.

#### Scenario: DNS page visible in Preferences
- **WHEN** the user opens Preferences
- **THEN** a "DNS" page SHALL appear alongside System, Network, and Routing pages

### Requirement: DNS master toggle
The DNS page SHALL have an enable/disable toggle at the top. When disabled, all other DNS controls SHALL be insensitive (grayed out).

#### Scenario: DNS disabled hides controls
- **WHEN** DNS is toggled off
- **THEN** all DNS server, rule, FakeIP, and advanced controls SHALL be insensitive

#### Scenario: DNS enabled activates controls
- **WHEN** DNS is toggled on
- **THEN** all DNS controls SHALL become interactive

### Requirement: DNS strategy selector
The DNS page SHALL have a dropdown to select the IP query strategy (Prefer IPv4, Prefer IPv6, IPv4 Only, IPv6 Only).

#### Scenario: Strategy selection persists
- **WHEN** the user selects "IPv6 Only" from the strategy dropdown
- **THEN** the setting SHALL be saved to AppSettings and reflected in the next config generation

### Requirement: DNS server list management
The DNS page SHALL display the list of configured DNS servers with controls to add, edit, and remove servers.

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

### Requirement: DNS rules management
The DNS page SHALL have a rules section with a toggle between auto-derived rules (from routing) and custom rules. When custom rules mode is active, the user SHALL be able to add, edit, and remove DNS rules.

#### Scenario: Auto-derived mode (default)
- **WHEN** the user has not enabled custom DNS rules
- **THEN** the rules section SHALL display a label indicating rules are auto-derived from routing, with no editable rule list

#### Scenario: Custom rules mode
- **WHEN** the user enables custom DNS rules
- **THEN** an editable rule list SHALL appear with add/edit/remove controls

#### Scenario: Add custom DNS rule
- **WHEN** the user adds a DNS rule
- **THEN** a dialog SHALL appear with match type (GeoSite, Domain Suffix), match value, and target server tag (dropdown of configured servers)

### Requirement: FakeIP section (sing-box conditional)
The DNS page SHALL show a FakeIP configuration section only when the selected backend is sing-box.

#### Scenario: FakeIP hidden for v2ray
- **WHEN** the selected backend is v2ray or xray
- **THEN** the FakeIP preferences group SHALL NOT be visible

#### Scenario: FakeIP shown for sing-box
- **WHEN** the selected backend is sing-box
- **THEN** the FakeIP group SHALL be visible with an enable toggle and IPv4/IPv6 range entries

### Requirement: Advanced DNS settings
The DNS page SHALL have an Advanced group with cache control toggle and EDNS client subnet entry.

#### Scenario: Disable cache toggle
- **WHEN** the user toggles "Disable DNS cache"
- **THEN** the disable_cache setting SHALL be saved and applied to config generation

#### Scenario: Client subnet entry
- **WHEN** the user enters a client subnet IP
- **THEN** the value SHALL be validated as a valid IPv4 or IPv6 address and saved

### Requirement: Static hosts table
The DNS page SHALL have a Hosts group where users can add and remove static domain→IP mappings.

#### Scenario: Add host override
- **WHEN** the user adds a host entry with domain "ads.example.com" and IP "127.0.0.1"
- **THEN** the entry SHALL appear in the hosts list and be persisted

#### Scenario: Remove host override
- **WHEN** the user removes a host entry
- **THEN** the entry SHALL be removed from the list and persisted immediately

### Requirement: Settings changes auto-persist
All DNS preference changes SHALL be saved immediately via the existing `on_settings_changed` callback pattern, consistent with other Preferences pages.

#### Scenario: Change persists without explicit save
- **WHEN** the user modifies any DNS setting
- **THEN** the change SHALL be persisted to settings.toml via the callback, without requiring a save button
