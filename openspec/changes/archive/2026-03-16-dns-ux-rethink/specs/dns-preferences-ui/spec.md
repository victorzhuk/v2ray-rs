## ADDED Requirements

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

---

## MODIFIED Requirements

### Requirement: DNS server list management (MODIFIED)
**Canonical location:** `dns-preferences-ui/spec.md` — "DNS server list management"

**Change:** The full DNS server list (add/edit/remove controls and the Providers button) has moved from the top-level DNS page into the Advanced section. The primary page surface now shows only the `remote` and `domestic` summary rows.

#### Scenario: Server list accessible via Advanced
- **WHEN** the user opens the DNS preferences page
- **THEN** the full editable server list is accessible by expanding the Advanced section, not at the top level of the page

### Requirement: DNS rules management (MODIFIED)
**Canonical location:** `dns-preferences-ui/spec.md` — "DNS rules management"

**Change:** The rules section (custom rules toggle and editable rule list) has moved from the top-level DNS page into the Advanced section.

#### Scenario: Rules section in Advanced
- **WHEN** the user wants to manage DNS routing rules
- **THEN** the rules section is accessible inside Advanced, not as a standalone section at the top level

### Requirement: Advanced DNS settings (MODIFIED)
**Canonical location:** `dns-preferences-ui/spec.md` — "Advanced DNS settings"

**Change:** The Advanced group now contains the full server list, rules section, hosts section, FakeIP settings, custom-rules toggle, and the existing cache/subnet controls, rather than only cache control and client subnet.

#### Scenario: Advanced contains all complex controls
- **WHEN** the user expands the Advanced section
- **THEN** it contains servers, rules, hosts, FakeIP, the custom-rules toggle, disable-cache toggle, and client-subnet entry

### Requirement: FakeIP section (MODIFIED)
**Canonical location:** `dns-preferences-ui/spec.md` — "FakeIP section (sing-box conditional)"

**Change:** FakeIP settings have moved from the top-level DNS page into the Advanced section. The sing-box-only visibility constraint is unchanged.

#### Scenario: FakeIP inside Advanced for sing-box
- **WHEN** the selected backend is sing-box and the user expands Advanced
- **THEN** the FakeIP group is visible inside Advanced, not as a top-level section

### Requirement: Static hosts table (MODIFIED)
**Canonical location:** `dns-preferences-ui/spec.md` — "Static hosts table"

**Change:** The Hosts group has moved from the top-level DNS page into the Advanced section.

#### Scenario: Hosts group inside Advanced
- **WHEN** the user wants to manage static host overrides
- **THEN** the Hosts group is accessible inside Advanced, not as a standalone section at the top level
