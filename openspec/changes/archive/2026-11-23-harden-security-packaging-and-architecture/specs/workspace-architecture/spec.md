# Spec Delta: workspace-architecture

## RENAMED Requirements

- FROM: `### Requirement: Eight-crate workspace DAG and pure core state types`
- TO: `### Requirement: Nine-crate workspace DAG and pure core state types`

## MODIFIED Requirements

### Requirement: Nine-crate workspace DAG and pure core state types
The workspace SHALL maintain a 9-crate layout (`core`, `tun-session`, `netctl`, `run`, `antidpi`, `subscription`, `process`, `tray`, `ui`) where `v2ray-rs-tray` depends on `v2ray-rs-core` without `v2ray-rs-process`, `v2ray-rs-antidpi` stays a standalone external binary with zero reverse crate dependents, and `v2ray-rs-subscription`'s dependency on `v2ray-rs-process` is explicitly documented for real-delay probing.

#### Scenario: Tray crate compiles without process crate dependency
- **WHEN** the workspace dependency graph is inspected via `cargo metadata --no-deps`
- **THEN** `v2ray-rs-tray` SHALL depend on `v2ray-rs-core` and SHALL NOT list `v2ray-rs-process` as a dependency

#### Scenario: Antidpi crate stays an external binary with no reverse Cargo dependents
- **WHEN** `cargo metadata --no-deps` is inspected
- **THEN** no workspace crate SHALL list `v2ray-rs-antidpi` as a Cargo dependency

#### Scenario: Existing process imports remain compatible
- **WHEN** a downstream crate imports previously public process state or log types from `v2ray-rs-process`
- **THEN** those paths SHALL resolve to the corresponding core types without caller changes

## ADDED Requirements

### Requirement: Focused process crate module decomposition
Monolithic modules in `crates/process/src/` (`session/mod.rs`, `manager.rs`, `privilege.rs`) SHALL be split into focused submodules separating lifecycle orchestration, privilege/mount probing, elevated script generation, and test fixtures while preserving public exports.

#### Scenario: Process submodules isolate responsibilities without API breaks
- **WHEN** `crates/process/src/` is organized into `session/`, `manager/`, and `privilege/` submodules
- **THEN** all existing public types and functions SHALL remain re-exported at their current crate paths and all process tests SHALL pass
