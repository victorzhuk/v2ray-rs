## MODIFIED Requirements

### Requirement: TUN configuration page
The system SHALL present a TUN configuration page in the preferences dialog with
an enable toggle, interface name, MTU, and address fields, plus an advanced
section. Field input SHALL be validated and SHALL NOT persist invalid values. The
advanced section SHALL expose three exclusion lists — excluded routes (CIDR),
excluded domains (suffix), and excluded applications (process name) — each gated
to the backends that support it.

#### Scenario: Primary TUN fields
- **WHEN** the user opens the TUN page with a TUN-capable backend selected
- **THEN** the page SHALL show an enable switch, an interface-name entry, an MTU spin control, and an IPv4 address (CIDR) entry, and invalid entries SHALL be rejected with an error indication without being saved

#### Scenario: Advanced TUN fields
- **WHEN** the user expands the advanced section
- **THEN** the page SHALL expose stack, strict route, DNS hijack mode, an excluded-routes (CIDR) list, an excluded-domains list, and an excluded-applications list, validating CIDR and domain entries before saving, with the excluded-routes and excluded-domains lists applying to both backends and the excluded-applications list applying to sing-box only, marking rows that do not apply to the active backend as insensitive with a note
