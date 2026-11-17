# Proposal

## Why

Domain orchestration, DNS derivation, and state persistence currently leak across crate and module boundaries: `v2ray-rs-tray` pulls in the entire `v2ray-rs-process` crate (`nix`, `tokio::process`, TUN/PID supervision) just to read `ProcessState`, `crates/ui/src/connection.rs` embeds ~1,650 lines of headless session supervision inside the GTK crate, DNS derivation is triplicated across `v2ray.rs`/`singbox.rs`/`effective_dns.rs`, `ConfigGenerator::generate` relies on an implicit positional `nodes[0]`/`nodes[1..]` slice, UI components bypass `WorkspaceStore` with unlocked main-thread JSON/TOML I/O, and `app.rs` (~5,000 LOC), `subscriptions.rs` (~2,665 LOC), and `preferences/dns.rs` (~2,068 LOC) have grown into god-modules.

## What Changes

- Move pure process state and log value types (`ProcessState`, `ProcessEvent`, `TransitionError`, `LogLine`, `LogSource`, `LogBuffer`) into `v2ray-rs-core` (re-exporting in `v2ray-rs-process`) and drop `v2ray-rs-process` from `v2ray-rs-tray`.
- Extract headless connection supervision, candidate failover, TUN hostname pinning/probing, sing-box connected-subnet snapshot orchestration, connection health monitoring, log batching, and failure classification from `crates/ui/src/connection.rs` into `v2ray-rs-process`, keeping `v2ray-rs-process` free of HTTP/TLS/gRPC dependencies while leaving a thin Relm4 bridge in `v2ray-rs-ui`.
- Trim `v2ray-rs-subscription` dependencies by configuring `tonic` as client-only (`default-features = false`, `transport`/`codegen`/`prost`), replacing `futures-util = "0.3"` with `futures.workspace = true`, and removing `tonic` types from public `ObservatoryError` variants.
- Introduce `ResolvedDnsPlan` and `ResolvedNodeSet { primary, detour_nodes }` in `v2ray-rs-core` so `v2ray.rs`, `singbox.rs`, and `effective_dns.rs` share a single DNS derivation pass and `ConfigGenerator::generate` enforces primary vs detour nodes in types.
- Upgrade `WorkspaceStore` into the single authoritative `Arc`-snapshot and serialized async persistence owner across `App`, `SubscriptionsPage`, `NodesPage`, `OnboardingWizard`, `PreferencesDialog`, and `geodata_service`.
- Decompose `crates/ui/src/app.rs`, `crates/ui/src/subscriptions.rs`, and `crates/ui/src/preferences/dns.rs` into focused submodules and update `AGENTS.md` architecture documentation to match.

## Capabilities

### New Capabilities

- `workspace-architecture`: Crate dependency boundaries, headless session supervision in `v2ray-rs-process`, shared `ResolvedDnsPlan`/`ResolvedNodeSet` contracts in `v2ray-rs-core`, client-only `subscription` dependencies, authoritative `WorkspaceStore` state ownership, and modular `v2ray-rs-ui` structure.

### Modified Capabilities

None.

## Impact

Affects `Cargo.toml`, `crates/core`, `crates/process`, `crates/subscription`, `crates/tray`, `crates/ui`, and `AGENTS.md`. Preserves the 8-crate workspace count with zero new crates and reduces transitive dependency compilation in `v2ray-rs-subscription` and `v2ray-rs-tray`.
