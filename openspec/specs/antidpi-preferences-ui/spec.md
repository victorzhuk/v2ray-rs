# antidpi-preferences-ui Specification

## Purpose

TBD - created by archiving change add-antidpi-integration. Update Purpose after archive.

## Requirements

### Requirement: Anti-DPI configuration page
The system SHALL present an Anti-DPI configuration page in the preferences dialog with an enable toggle, a queue number field, a strategy list, and an optional hostlist path field. Field input SHALL be validated and SHALL NOT persist invalid values. The page SHALL state that the engine treats the early packets of TLS and QUIC flows to port 443 on host egress, that it works with every backend and with TUN on or off, that a configured hostlist applies to TLS (and HTTP Host) only, that changes apply on the next connect, and SHALL NOT claim coverage for plain HTTP or guarantee that the first packets of a connection attempt were processed.

#### Scenario: Primary anti-DPI fields
- **WHEN** the user opens the Anti-DPI page
- **THEN** the page SHALL show an enable switch, a queue spin control defaulting to 200, a strategy list, and an optional hostlist path entry, and invalid entries SHALL be rejected with an error indication without being saved

#### Scenario: Empty strategy list means helper defaults
- **WHEN** the user enables anti-DPI without adding any strategy
- **THEN** the page SHALL state that the helper's built-in default strategies apply

#### Scenario: Invalid queue or strategy is not saved
- **WHEN** the user enters queue `0` or an empty strategy string
- **THEN** the page SHALL reject the entry with an error indication and the persisted settings SHALL keep the previous value

### Requirement: Strategy validation is shape-only on the page
The page SHALL validate strategy strings by shape only — non-empty, bounded length, and bounded count, via the settings model's validator — and SHALL NOT persist invalid values. The strategy chain DSL itself SHALL be validated by the helper's `check --strategy` subcommand and, decisively, at connect time: a chain the helper's parser rejects SHALL fail the connection attempt's `install-rules` invocation with the helper's error. Strategy entries edited on the page are accepted as-is when they pass the shape checks.

#### Scenario: Malformed-by-shape strategy is not saved
- **WHEN** the user saves more strategy chains than the configured maximum, or a chain exceeding the length bound, or a blank hostlist path
- **THEN** the page SHALL reject the entry with an error indication and the persisted settings SHALL keep the previous value

#### Scenario: Helper-side validation happens at connect
- **WHEN** the user saves a strategy chain that passes the shape checks but the helper's parser rejects
- **THEN** the settings SHALL persist, and the next connection attempt SHALL fail the one-shot `install-rules` invocation with the helper's error before any backend is spawned

### Requirement: Capability hint on the page
The page SHALL show a capability hint row naming the capability pair (`cap_net_admin`, `cap_net_raw`) the engine needs and pointing at the "Grant TUN privileges" action on the TUN page, which covers the anti-DPI helper in the same elevation. The page itself SHALL NOT offer an inline grant button. The TUN page's shared capability row SHALL also probe the resolved anti-DPI helper and SHALL make the grant action visible when the anti-DPI helper alone needs a capability, enrollment, permission, or stale-copy repair, so the hinted repair is always reachable.

#### Scenario: Capability hint names the grant path
- **WHEN** the user opens the Anti-DPI page
- **THEN** the page SHALL show a capability hint row naming the required capability pair and the TUN page's grant action as the one-elevation repair path

#### Scenario: Anti-DPI-only capability loss exposes the grant
- **WHEN** the backend and route helper are healthy but the anti-DPI helper lacks a required effective capability
- **THEN** the TUN page's capability row SHALL report the anti-DPI helper and show the grant action, and the next connect SHALL fail with an error naming the missing capability

#### Scenario: Connect surfaces a missing capability with the grant action
- **WHEN** anti-DPI is enabled but the helper binary lacks `cap_net_admin` or `cap_net_raw`
- **THEN** the connection SHALL fail with an error naming the missing capability, and the session SHALL surface the shared grant action for the single elevation

### Requirement: Editing anti-DPI settings while connected defers to restart
The page SHALL allow editing all anti-DPI fields while connected and SHALL surface the restart-required banner through the standard snapshot divergence flow, without silently restarting the session.

#### Scenario: Strategy edit while connected shows the banner
- **WHEN** the user adds or removes a strategy while connected
- **THEN** the settings SHALL persist and the restart-required banner SHALL appear; the running session's engine SHALL keep its current strategies until restart
