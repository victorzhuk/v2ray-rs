# Tasks

## 1. Core State Types, `ResolvedNodeSet`, and `ResolvedDnsPlan` (`v2ray-rs-core` & `v2ray-rs-tray`)

- [x] 1.1 Move `ProcessState`, `ProcessEvent`, `TransitionError`, `LogLine`, `LogSource`, and `LogBuffer` into `crates/core/src/`, re-export them in `crates/process/src/lib.rs`, update `crates/tray` to depend only on `v2ray-rs-core` (removing `v2ray-rs-process` from `crates/tray/Cargo.toml`), and verify `cargo metadata --no-deps` and `cargo test -p v2ray-rs-core -p v2ray-rs-tray` pass
- [x] 1.2 Replace `nodes: &[ProxyNode]` with `ResolvedNodeSet` in `ConfigGenerator::generate` and `ConfigWriter` in `crates/core/src/config/`, and verify all config generator unit tests compile and pass
- [x] 1.3 Implement `ResolvedDnsPlan` in `crates/core/src/config/dns_plan.rs` and refactor `v2ray.rs`, `singbox.rs`, and `effective_dns.rs` to lower from `ResolvedDnsPlan`, verifying generator golden/unit tests and `Effective DNS` equivalence tests pass

## 2. Subscription Dependency Slimming (`v2ray-rs-subscription`)

- [x] 2.1 Configure `tonic` with `default-features = false, features = ["transport", "codegen", "prost"]`, replace `futures-util` with `futures.workspace = true` in `crates/subscription/Cargo.toml`, and encapsulate `tonic` error types inside `ObservatoryError`, verifying `cargo tree -p v2ray-rs-subscription` shows no `axum`/server features and `cargo test -p v2ray-rs-subscription` passes

## 3. Headless Session Supervisor Extraction (`v2ray-rs-process`)

- [x] 3.1 Extract session supervision, candidate failover, `PinCache`/`os_lookup`/`probe_pins`, `connected_networks` snapshot coordination, `connection_health`, log batching, and `failure_classify` from `crates/ui/src/` into `crates/process/src/session/`, keeping `v2ray-rs-process` free of HTTP/TLS/gRPC crates, and verify headless `cargo test -p v2ray-rs-process` session failover/health/pinning tests pass
- [x] 3.2 Replace `crates/ui/src/connection.rs` with a thin Relm4 bridge over `v2ray_rs_process::session` and verify `cargo check -p v2ray-rs-ui` and UI connection state tests pass

## 4. Authoritative `WorkspaceStore` & UI Module Split (`v2ray-rs-ui`)

- [x] 4.1 Upgrade `crates/ui/src/store.rs` (`WorkspaceStore`) to own authoritative `Arc` snapshots (`AppSettings`, subscriptions, manual nodes, routing rules, `LatencySnapshot`) and serialized background persistence, migrating direct `persistence::*` reads/writes in `app`, `subscriptions`, `nodes`, `routing`, `wizard`, `preferences`, and `geodata_service`, and verifying concurrent store mutation tests pass
- [x] 4.2 Split `crates/ui/src/app.rs`, `crates/ui/src/subscriptions.rs`, and `crates/ui/src/preferences/dns.rs` into focused submodules (`app/{mod,bootstrap,session_bridge,config_regen,dialogs}.rs`, `subscriptions/{mod,rows,dialogs,probes}.rs`, `preferences/dns/{mod,servers,rules,hosts,effective}.rs`) and update `AGENTS.md` crate/module descriptions, verifying `cargo test --workspace` passes

## 5. Workspace Architecture Verification

- [x] 5.1 Run `cargo test --workspace` with bounded timeouts, verify crate DAG edges via `cargo tree --workspace --depth 1`, and run `openspec validate split-workspace-boundaries --strict`

## Workflow follow-up

- Archive the change once implementation and verification complete.
