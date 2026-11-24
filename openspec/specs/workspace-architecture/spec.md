# workspace-architecture Specification

## Purpose
Defines crate dependency boundaries, headless session supervision ownership, shared DNS and node config contracts, and centralized state persistence across the nine-crate workspace.

## Requirements

### Requirement: Nine-crate workspace DAG and pure core state types
The workspace SHALL maintain nine crates: `core`, `tun-session`, `netctl`, `run`, `antidpi`, `subscription`, `process`, `tray`, and `ui`. Direct normal Cargo dependencies SHALL match the workspace graph below. Executing a helper SHALL NOT imply a Cargo dependency on its crate.
`netctl` SHALL depend on `core` only for tests. The `subscription` dependency on `process` supports real-delay probing.

#### Scenario: Direct workspace dependencies match the graph
- **WHEN** the workspace Cargo dependencies are inspected
- **THEN** the direct normal dependencies SHALL match this graph:

| Crate | Direct normal workspace dependencies |
|---|---|
| `ui` | `core`, `process`, `subscription`, `tray` |
| `subscription` | `core`, `process` |
| `process`, `tray` | `core` |
| `core`, `netctl`, `antidpi` | `tun-session` |
| `run`, `tun-session` | none |

#### Scenario: Tray crate compiles without process crate dependency
- **WHEN** the workspace dependency graph is inspected via `cargo metadata --no-deps`
- **THEN** `v2ray-rs-tray` SHALL depend on `v2ray-rs-core` and SHALL NOT list `v2ray-rs-process` as a dependency

#### Scenario: Antidpi crate stays an external binary with no reverse Cargo dependents
- **WHEN** `cargo metadata --no-deps` is inspected
- **THEN** no workspace crate SHALL list `v2ray-rs-antidpi` as a Cargo dependency

#### Scenario: Existing process imports remain compatible
- **WHEN** a downstream crate imports previously public process state or log types from `v2ray-rs-process`
- **THEN** those paths SHALL resolve to the corresponding core types without caller changes

### Requirement: Anti-DPI helper owns its journal
The process crate SHALL invoke the anti-DPI helper as an external executable.
The helper SHALL own `antidpi.v1.json` independently of the TUN journal payload.
The two journals SHALL share identity types through `tun-session`.

#### Scenario: Live controller removes its own anti-DPI rules
- **WHEN** a live controller removes its anti-DPI rules
- **THEN** it SHALL use its owner and session identity without `--sweep`
- **AND** offline sweep SHALL refuse a live owner

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

### Requirement: Focused process crate module decomposition
Monolithic modules in `crates/process/src/` (`session/mod.rs`, `manager.rs`, `privilege.rs`) SHALL be split into focused submodules separating lifecycle orchestration, privilege/mount probing, elevated script generation, and test fixtures while preserving public exports.

#### Scenario: Process submodules isolate responsibilities without API breaks
- **WHEN** `crates/process/src/` is organized into `session/`, `manager/`, and `privilege/` submodules
- **THEN** all existing public types and functions SHALL remain re-exported at their current crate paths and all process tests SHALL pass
