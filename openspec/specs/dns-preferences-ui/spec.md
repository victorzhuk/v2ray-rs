## Purpose

Provide a Preferences UI for enabling, viewing, and editing DNS servers, strategy, rules, FakeIP, hosts, and advanced settings.

## Requirements

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

### Requirement: Simplified default DNS projection
The DNS page SHALL show dedicated `remote` and `domestic` rows as the primary surface, while keeping the full server list available in Advanced.

#### Scenario: Standard config shows remote and domestic rows
- **WHEN** the user opens the DNS preferences page with standard `remote` and `domestic` servers configured
- **THEN** the primary section shows those two rows first

#### Scenario: Config without standard tags
- **WHEN** the current DNS config lacks a `remote` or `domestic` server tag
- **THEN** the primary row shows "Not configured" and the full editable server list remains available in Advanced without silent tag normalization

#### Scenario: Custom rules active indicator in primary view
- **WHEN** `use_custom_rules` is true
- **THEN** the primary DNS section SHALL indicate that custom DNS rules are active (e.g. via subtitle text or an info row), so the user is aware without expanding Advanced

### Requirement: Advanced DNS controls
Advanced DNS controls SHALL contain the full server list, the `use_custom_rules` toggle, the editable custom-rule list, FakeIP settings, and sing-box-only detour fields.

#### Scenario: Enable custom rules in Advanced
- **WHEN** the user expands Advanced and enables custom DNS rules
- **THEN** the existing `use_custom_rules` behavior becomes active and the editable custom-rule list is shown

#### Scenario: Extra servers remain editable in Advanced
- **WHEN** the DNS config contains additional nonstandard servers
- **THEN** the primary section continues to show only the standard roles and Advanced retains the full server list unchanged

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

#### Scenario: Server list accessible via Advanced
- **WHEN** the user opens the DNS preferences page
- **THEN** the full editable server list is accessible by expanding the Advanced section, not at the top level of the page

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

#### Scenario: Rules section in Advanced
- **WHEN** the user wants to manage DNS routing rules
- **THEN** the rules section is accessible inside Advanced, not as a standalone section at the top level

### Requirement: FakeIP section (sing-box conditional)
The DNS page SHALL show a FakeIP configuration section only when the selected backend is sing-box.

#### Scenario: FakeIP hidden for v2ray
- **WHEN** the selected backend is v2ray or xray
- **THEN** the FakeIP preferences group SHALL NOT be visible

#### Scenario: FakeIP shown for sing-box
- **WHEN** the selected backend is sing-box
- **THEN** the FakeIP group SHALL be visible with an enable toggle and IPv4/IPv6 range entries

#### Scenario: FakeIP inside Advanced for sing-box
- **WHEN** the selected backend is sing-box and the user expands Advanced
- **THEN** the FakeIP group is visible inside Advanced, not as a top-level section

### Requirement: Advanced DNS settings
The DNS page SHALL have an Advanced section containing the full server list, rules section, hosts section, FakeIP settings, the custom-rules toggle, disable-cache toggle, and client-subnet entry.

#### Scenario: Disable cache toggle
- **WHEN** the user toggles "Disable DNS cache"
- **THEN** the disable_cache setting SHALL be saved and applied to config generation

#### Scenario: Client subnet entry
- **WHEN** the user enters a client subnet IP
- **THEN** the value SHALL be validated as a valid IPv4 or IPv6 address and saved

#### Scenario: Advanced contains all complex controls
- **WHEN** the user expands the Advanced section
- **THEN** it contains servers, rules, hosts, FakeIP, the custom-rules toggle, disable-cache toggle, and client-subnet entry

### Requirement: Static hosts table
The DNS page SHALL have a Hosts group where users can add and remove static domain→IP mappings.

#### Scenario: Add host override
- **WHEN** the user adds a host entry with domain "ads.example.com" and IP "127.0.0.1"
- **THEN** the entry SHALL appear in the hosts list and be persisted

#### Scenario: Remove host override
- **WHEN** the user removes a host entry
- **THEN** the entry SHALL be removed from the list and persisted immediately

#### Scenario: Hosts group inside Advanced
- **WHEN** the user wants to manage static host overrides
- **THEN** the Hosts group is accessible inside Advanced, not as a standalone section at the top level

### Requirement: Settings changes auto-persist
All DNS preference changes SHALL be saved immediately via the existing `on_settings_changed` callback pattern, consistent with other Preferences pages.

#### Scenario: Change persists without explicit save
- **WHEN** the user modifies any DNS setting
- **THEN** the change SHALL be persisted to settings.toml via the callback, without requiring a save button

### Requirement: Downgrade warning in the DNS server dialog
The DNS server dialog SHALL keep every protocol selectable regardless of the active backend, and SHALL show an inline warning when the selected protocol will be downgraded for the active backend, naming the effective protocol. The warning SHALL be derived from the same core compatibility function used by the config generators.

#### Scenario: Selecting an unsupported protocol warns inline
- **WHEN** the active backend is v2ray and the user selects DoT, DoQ, or H3 (or the backend is xray and the user selects H3) in the server dialog
- **THEN** an inline warning SHALL state that the server will run as DoH on the active backend, and saving SHALL still be allowed

#### Scenario: Supported protocol shows no warning
- **WHEN** the selected protocol is natively supported by the active backend
- **THEN** no downgrade warning SHALL be shown

### Requirement: Downgrade indicator on saved DNS server rows
The DNS server list SHALL passively mark saved servers whose configured protocol the active backend will not honor, so a backend switch cannot leave a silent downgrade invisible.

#### Scenario: Backend switch reveals affected servers
- **WHEN** the active backend changes and a saved DNS server's protocol will be downgraded on it
- **THEN** that server's row SHALL show a downgrade indicator naming the effective protocol

#### Scenario: Stored protocol is preserved
- **WHEN** a saved server's protocol is unsupported by the active backend
- **THEN** the stored protocol value SHALL remain unchanged unless the user edits it

### Requirement: Programmatic widget updates do not re-enter handlers
When the DNS preferences page updates its own widgets to reflect a settings change it made, those updates SHALL NOT re-enter the widgets' own change handlers, and no borrow of the shared settings state SHALL be held across a widget setter. GTK emits property notifications synchronously, so a borrow held across a setter re-enters the handler and aborts the process rather than raising a recoverable error.

#### Scenario: Applying a provider preset re-syncs the page
- **WHEN** the user applies a DNS provider preset while the IP strategy is set to something other than the preset's
- **THEN** the master enable switch and the strategy row SHALL both update to the preset's values, the settings SHALL be mutated once, and the application SHALL NOT abort

#### Scenario: Suppressed handlers do not write settings back
- **WHEN** the page drives a widget programmatically
- **THEN** the widget's change handler SHALL make no settings mutation and emit no settings change for that update

### Requirement: Detour is configurable for every backend that honors it
The detour control in the DNS server dialog SHALL retain the chosen value for every backend that can act on it — sing-box, which emits it on the server object, and xray, which expresses a direct detour as a routing rule. The value SHALL NOT be discarded on save for those backends.

#### Scenario: Detour is retained for xray
- **WHEN** the user sets a server's detour to direct while the backend is xray and saves
- **THEN** the saved server SHALL keep that detour instead of having it cleared

#### Scenario: Detour stays available for sing-box
- **WHEN** the backend is sing-box
- **THEN** the detour control SHALL behave as before

### Requirement: Detour applicability is explained per backend
The detour control in the DNS server dialog SHALL state its effect for the selected backend. For xray it SHALL state that only `direct` has an effect and only while TUN is enabled. For sing-box it SHALL state that `proxy` sends the server through the proxy and `direct` dials it directly.

#### Scenario: xray note
- **WHEN** the backend is xray and the user opens the DNS server dialog
- **THEN** the detour row SHALL state that only `direct` applies, and only with TUN enabled

#### Scenario: sing-box note
- **WHEN** the backend is sing-box and the user opens the DNS server dialog
- **THEN** the detour row SHALL state that `proxy` routes the server through the proxy and `direct` dials it directly

### Requirement: Warning for private DNS servers routed through the proxy
The DNS server dialog SHALL show a non-blocking inline warning while the entered address, detour, and active backend make the server flagged as a private DNS server routed through the proxy, stating that queries will reach the proxy server's network and suggesting detour `direct`. The server's row in the list and in the primary section SHALL show the same warning. Saving SHALL remain allowed.

#### Scenario: Warning while editing
- **WHEN** the active backend is sing-box and the user enters address `127.0.0.1` with detour `proxy`
- **THEN** the dialog SHALL show the warning and the Save response SHALL stay enabled

#### Scenario: Warning clears on direct detour
- **WHEN** the user changes that server's detour to `direct`
- **THEN** the warning SHALL disappear

#### Scenario: Saved flagged server marked
- **WHEN** a saved server is flagged for the active backend
- **THEN** its row SHALL show the warning text

### Requirement: Strategy options reflect the backend
The IP strategy selector SHALL keep its four options and stored values, and when the active backend is xray or v2ray SHALL show a note that "Prefer IPv4" and "Prefer IPv6" query only the preferred address family on that backend. For sing-box no note SHALL be shown.

#### Scenario: xray strategy note
- **WHEN** the active backend is xray and the DNS page is shown
- **THEN** the strategy row SHALL show the note that Prefer options use only the preferred family

#### Scenario: sing-box shows no note
- **WHEN** the active backend is sing-box
- **THEN** the strategy row SHALL show no family note

### Requirement: Missing auto-split tag is shown
While custom DNS rules are off, the primary `remote` or `domestic` row whose tag has no configured server SHALL state that DNS rules derived from routing for that server are skipped, instead of only "Not configured".

#### Scenario: Domestic tag missing in auto mode
- **WHEN** `use_custom_rules` is false and no server is tagged `domestic`
- **THEN** the Domestic row SHALL state that routing-derived DNS rules for direct traffic are skipped

#### Scenario: Custom rules active
- **WHEN** `use_custom_rules` is true and no server is tagged `domestic`
- **THEN** the Domestic row SHALL show "Not configured" without the skipped-rules note

### Requirement: DNS rule keyword values are validated
The DNS rule dialog SHALL validate a Domain Keyword value with the same rule as routing keyword rules and SHALL NOT save a value containing `*` or whitespace, showing the validation message inline. Stored DNS keyword rules with such a value SHALL stay unchanged. Their rows SHALL be marked invalid with error styling and use the validation message as their subtitle.

#### Scenario: Wildcard DNS keyword rejected
- **WHEN** the user adds a DNS rule with match type Domain Keyword and value `*.cn`
- **THEN** the dialog SHALL show an inline error and the rule SHALL NOT be saved

#### Scenario: Stored wildcard DNS keyword surfaced
- **WHEN** stored settings contain a DNS keyword rule `*.cn`
- **THEN** the settings SHALL load unchanged and the rule's row SHALL show the validation error as its subtitle

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
