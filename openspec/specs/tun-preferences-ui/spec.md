# Spec: TUN Preferences UI

## Purpose

Defines the TUN configuration page in the preferences dialog: field layout, validation, backend/capability gating, and the system-wide routing warning shown when TUN is first enabled.

## Requirements

### Requirement: TUN configuration page
The system SHALL present a TUN configuration page in the preferences dialog with an enable toggle, interface name, MTU, and address fields, plus an advanced section. Field input SHALL be validated and SHALL NOT persist invalid values.

#### Scenario: Primary TUN fields
- **WHEN** the user opens the TUN page with a TUN-capable backend selected
- **THEN** the page SHALL show an enable switch, an interface-name entry, an MTU spin control, and an IPv4 address (CIDR) entry, and invalid entries SHALL be rejected with an error indication without being saved

#### Scenario: Advanced TUN fields
- **WHEN** the user expands the advanced section
- **THEN** the page SHALL expose stack, strict route, DNS hijack mode, and an excluded-routes list, marking rows that do not apply to the active backend as insensitive with a note

### Requirement: Capability and backend gating in the UI
The TUN page SHALL reflect backend support and capability state and SHALL offer the privilege grant inline.

#### Scenario: Grant action when capabilities are missing
- **WHEN** TUN is enabled but the backend binary lacks `CAP_NET_ADMIN`
- **THEN** the page SHALL show a "Grant TUN privileges" button that triggers the one-time `pkexec` grant and refreshes the displayed capability state when it completes

#### Scenario: TUN unavailable for v2ray
- **WHEN** the active backend is v2ray
- **THEN** the enable toggle SHALL be insensitive with a note that TUN requires sing-box or xray

### Requirement: System-wide routing warning
The system SHALL warn the user that enabling TUN routes all system traffic through the active proxy.

#### Scenario: Warning on enable
- **WHEN** the user switches TUN on
- **THEN** the UI SHALL display a one-shot warning toast stating that all system traffic will be routed through the active proxy
