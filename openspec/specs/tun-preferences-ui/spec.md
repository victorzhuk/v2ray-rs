# Spec: TUN Preferences UI

## Purpose

Defines the TUN configuration page in the preferences dialog: field layout, validation, backend/capability gating, and the system-wide routing warning shown when TUN is first enabled.

## Requirements

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

### Requirement: Exclusion lists describe their effect per backend
The excluded-routes and excluded-domains groups SHALL describe what each list does on the active backend. Excluded routes SHALL be described as destinations routed outside the tunnel on both backends. For xray, excluded domains SHALL be described as traffic that still enters the tunnel and is sent directly by xray after it recognizes the domain, so they do not keep traffic off the TUN device; for sing-box, the description SHALL describe domain suffixes that bypass the tunnel. Both groups SHALL state that changes apply on the next connect.

#### Scenario: xray domain exclusion wording
- **WHEN** the active backend is xray and the TUN page is shown
- **THEN** the excluded-domains description SHALL state that matching traffic still passes through the tunnel and is sent directly by xray

#### Scenario: Route exclusion wording
- **WHEN** the active backend is xray or sing-box
- **THEN** the excluded-routes description SHALL state that the CIDRs are routed outside the tunnel and that changes apply on the next connect
