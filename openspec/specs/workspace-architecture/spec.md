# workspace-architecture Specification

## Purpose
Defines crate dependency boundaries, headless session supervision ownership, shared DNS and node config contracts, and centralized state persistence across the 8-crate workspace.

## Requirements

### Requirement: Eight-crate workspace DAG and pure core state types
The workspace SHALL retain its 8-crate layout (`core`, `tun-session`, `netctl`, `run`, `subscription`, `process`, `tray`, `ui`) while defining pure state and log types (`ProcessState`, `ProcessEvent`, `TransitionError`, `LogLine`, `LogSource`, `LogBuffer`) in `v2ray-rs-core` so `v2ray-rs-tray` depends on `v2ray-rs-core` without depending on `v2ray-rs-process`.

#### Scenario: Tray crate compiles without process crate dependency
- **WHEN** the workspace dependency graph is inspected via `cargo metadata --no-deps`
- **THEN** `v2ray-rs-tray` SHALL depend on `v2ray-rs-core` and SHALL NOT list `v2ray-rs-process` as a dependency

#### Scenario: Existing process imports remain compatible
- **WHEN** a downstream crate imports previously public process state or log types from `v2ray-rs-process`
- **THEN** those paths SHALL resolve to the corresponding core types without caller changes

### Requirement: Headless connection session supervisor lives in process crate
Headless connection session supervision -- candidate failover, TUN hostname pin resolution and probing, sing-box connected-subnet snapshot orchestration, connection health monitoring, log batching, and failure classification -- SHALL reside in `v2ray-rs-process` without adding HTTP, TLS, or gRPC dependencies to `v2ray-rs-process`, leaving `v2ray-rs-ui` as a Relm4 event bridge.

#### Scenario: Headless session failover tested without GTK initialization
- **WHEN** `cargo test -p v2ray-rs-process` runs in a headless environment with no display server
- **THEN** candidate failover, TUN pin caching, health probe state transitions, and failure classification SHALL execute directly inside `v2ray-rs-process` without linking GTK4 or `reqwest`/`tonic`

### Requirement: Client-only gRPC surface and opaque errors in subscription crate
`v2ray-rs-subscription` SHALL configure `tonic` with client-only features (omitting server and router features), reuse `futures.workspace = true` instead of a separate `futures-util` manifest entry, and keep `tonic` transport and status types private behind `ObservatoryError`.

#### Scenario: Subscription public API leaks no tonic types
- **WHEN** downstream crates (`v2ray-rs-ui`) match or format `ObservatoryError`
- **THEN** no public variant or method signature of `v2ray-rs-subscription` SHALL expose `tonic::Status` or `tonic::transport::Error`

#### Scenario: The subscription build enables only client transport
- **WHEN** Cargo resolves the normal dependency features for `v2ray-rs-subscription`
- **THEN** tonic's `channel` feature SHALL be enabled and its `server` and `router` features SHALL be disabled

### Requirement: Single-source DNS plan and typed node set in config generation
`v2ray-rs-core` SHALL compute one `ResolvedDnsPlan` shared by `v2ray`/`xray` config generation, `sing-box` config generation, and `Effective DNS` derivation, and `ConfigGenerator::generate` SHALL accept a typed `ResolvedNodeSet` (`primary` plus `detour_nodes`) rather than an implicit positional node slice.

#### Scenario: Generators and Effective DNS consume the same ResolvedDnsPlan
- **WHEN** DNS settings, TUN exclusions, and routing rules are evaluated for a backend
- **THEN** `v2ray`/`xray`, `sing-box`, and `effective_dns` SHALL all derive their emitted servers and scoping from the same `ResolvedDnsPlan` value, and `ConfigGenerator::generate` SHALL reject empty node sets at the type level

### Requirement: Authoritative WorkspaceStore owns state snapshots and async persistence
`WorkspaceStore` SHALL hold authoritative `Arc`-shared snapshots of `AppSettings`, subscriptions, manual nodes, routing rules, and latency metadata and serialize all disk writes and corrupt-store resets (including resetting unreadable `routing_rules.json` to an empty rule set) off the GTK main thread, eliminating direct unlocked filesystem mutations in UI components.

#### Scenario: Concurrent UI mutations serialize through WorkspaceStore without clobbering
- **WHEN** Preferences edits `AppSettings` while a background subscription refresh or wizard completion updates subscriptions and manual nodes
- **THEN** all mutations SHALL flow through `WorkspaceStore` in order, broadcast updated `Arc` snapshots to subscribers, and persist atomically off the GTK main thread without stale-file overwrites

#### Scenario: Corrupt routing rules reset flows through WorkspaceStore
- **WHEN** `routing_rules.json` fails to load on startup and the user confirms "Reset to Empty" on the routing error page
- **THEN** `WorkspaceStore` SHALL atomically persist an empty `RoutingRuleSet` on its serialized worker thread, clear the stored routing load error, and broadcast the empty snapshot without direct `std::fs::remove_file` calls on the GTK thread

### Requirement: Focused UI module decomposition
Multi-concern UI god-modules (`crates/ui/src/app.rs`, `crates/ui/src/subscriptions.rs`, and `crates/ui/src/preferences/dns.rs`) SHALL be split into focused submodules separating CLI/app bootstrap, session controller bridging, list/row presentation, editor dialogs, and probe actions.

#### Scenario: UI submodules isolate bootstrap, session bridge, lists, and editors
- **WHEN** `crates/ui/src/` is organized after decomposition
- **THEN** `app`, `subscriptions`, and `preferences/dns` SHALL each separate state/update orchestration from dialog builders and widget helpers with no behavioral regressions
