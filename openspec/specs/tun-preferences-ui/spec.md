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
The excluded-routes and excluded-domains groups SHALL describe what each list does on the active backend. Excluded routes SHALL be described as destinations routed outside the tunnel on both backends, and SHALL state that the host's own attached networks are excluded automatically. For xray, excluded domains SHALL be described as traffic that still enters the tunnel and is sent directly by xray after it recognizes the domain, so they do not keep traffic off the TUN device; for sing-box, the description SHALL describe domain suffixes that bypass the tunnel. Both groups SHALL state that changes apply on the next connect.

#### Scenario: xray domain exclusion wording
- **WHEN** the active backend is xray and the TUN page is shown
- **THEN** the excluded-domains description SHALL state that matching traffic still passes through the tunnel and is sent directly by xray

#### Scenario: Route exclusion wording
- **WHEN** the active backend is xray or sing-box
- **THEN** the excluded-routes description SHALL state that the CIDRs are routed outside the tunnel, that the host's own attached networks are excluded automatically, and that changes apply on the next connect

### Requirement: DNS hijack mode effect is explained
The TUN page SHALL explain the DNS hijack modes next to the selector: `hijack` routes DNS queries captured by the tunnel to the backend's resolver, while `native` and `disabled` both leave DNS queries to be carried as ordinary traffic without being answered by the backend's resolver.

#### Scenario: Hijack row note
- **WHEN** the user opens the TUN page with sing-box or xray selected
- **THEN** the DNS hijack row SHALL state that `native` and `disabled` currently behave the same and do not hand DNS queries to the backend's resolver

### Requirement: TUN preferences guard rollback re-entrancy and parse Run-with-bypass argv
Programmatic widget rollbacks in TUN preferences SHALL suppress re-entrant signal handlers via a guard flag, `v2ray-rs-bypass` account lookup SHALL run off the GTK main thread, and `Run with bypass` SHALL parse command lines with `glib::shell_parse_argv` and report non-zero child exit statuses to the user.

#### Scenario: Quoted arguments in Run with bypass are preserved
- **WHEN** the user runs `curl -H "Accept: application/json" https://example.com` via `Run with bypass`
- **THEN** `v2ray-rs-run` SHALL receive `["curl", "-H", "Accept: application/json", "https://example.com"]` without splitting inside quotes

#### Scenario: Failed bypass command surfaces error toast
- **WHEN** a command launched via `Run with bypass` exits immediately with a non-zero status or spawn error
- **THEN** the UI SHALL surface an error toast naming the failure

### Requirement: Grant confirmation verifies expected post-grant paths and respects targeting
`grant_confirmed` SHALL compare post-grant probe paths against expected post-grant destination paths (relocated under `/usr/local/lib/v2ray-rs` when relocation applies) and SHALL only require `AntidpiVerdict::Ready` when the anti-DPI helper was targeted by the grant.

#### Scenario: First-time AppImage relocating grant confirms cleanly
- **WHEN** a user grants privileges from a `nosuid` AppImage mount and both helpers relocate to `/usr/local/lib/v2ray-rs/`
- **THEN** `grant_confirmed` SHALL compare against the relocated paths and return `true`

#### Scenario: Untargeted fixable anti-DPI helper does not fail a TUN-only grant
- **WHEN** `grant_confirmed` runs with `antidpi_targeted == false` and the anti-DPI probe returns `AntidpiVerdict::Fixable`
- **THEN** `grant_confirmed` SHALL return `true`

### Requirement: TUN preferences avoid widget reference cycles, main-thread probes, and untranslated verdicts
`build_tun_page` SHALL hold a `Weak` reference in its refresh closure to avoid leaking widgets, SHALL resolve helper and wrapper paths off the GTK main thread, and SHALL provide gettext catalog entries for every `tr(...)` capability verdict string.

#### Scenario: Helper path resolution runs off the GTK main thread
- **WHEN** the user opens TUN preferences or clicks Grant
- **THEN** `run_path()`, `helper_path()`, and `antidpi_path()` SHALL run inside `spawn_blocking` rather than synchronously on the GTK thread

#### Scenario: All anti-DPI capability verdicts translate in ru_RU
- **WHEN** the Russian `.mo` catalog is inspected for the four anti-DPI verdict strings from `preferences/tun.rs`
- **THEN** every `msgid` SHALL be present with a Russian translation

### Requirement: Strategy editor rejects multi-newline blank inputs
`strategies_from_text` SHALL treat only `""` and a single trailing `"\n"` as clearing the saved strategy list and SHALL reject multi-newline blank inputs such as `"\n\n"` as invalid blank lines.

#### Scenario: Double newline does not clear saved strategies
- **WHEN** the user enters `"\n\n"` in the strategy editor and focuses out
- **THEN** `strategies_from_text` SHALL return an error and keep the previously saved strategy list
