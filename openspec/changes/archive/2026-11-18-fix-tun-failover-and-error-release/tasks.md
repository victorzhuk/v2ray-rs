# Tasks

## 1. Failover TUN Ownership and Route Reconciliation

- [x] 1.1 Update `ProcessManager` and `crates/process/src/session/mod.rs` so a parked failed candidate whose `(backend, iface, session_id)` was adopted by a `Running` replacement candidate drops without running `teardown_tun` (`xray-down` or `verify_singbox_stopped`), and verify with unit/session failover tests covering pre-device failure, post-device failure, and sing-box failover.
- [x] 1.2 Reconcile journaled priority-8998 (`--bypass-uid`) and priority-8999 (`--capture-dns`) policy rules during `xray-up` in `crates/netctl`, and verify with unit and namespace tests (`crates/netctl`) that re-running `xray-up` without `--capture-dns` or `--bypass-uid` removes the prior rules from kernel state and the journal.
- [x] 1.3 Update `docs/ARCHITECTURE.md` if needed to document session-adopted parked manager disposal and rule reconciliation, and verify documented priority behavior matches `crates/netctl`.

## 2. Terminal-Error TUN Release and Probe Gating in UI

- [x] 2.1 Update `crates/ui/src/app/mod.rs` so preflight aborts on follow-up `Connect` / `ConnectToNode` from `Error` release any present TUN session marker, and non-TUN connects while `tun_marker_present()` wait for `release_tun_session` before spawning; verify with `v2ray-rs-ui` state-transition tests covering `Apply & Restart` after disabling TUN, switching to `v2ray`, and preflight errors (empty candidates, missing geodata, disabled IPv6).
- [x] 2.2 Update `tun_active_for` in `crates/ui/src/app/config_regen.rs` and `SetTunActive` forwarding in `crates/ui/src/app/mod.rs` (including `TunSessionReleased`) to cover `Starting`, `Running`, `Stopping`, `tun_release_in_flight`, and fail-closed `Error` with a present TUN marker; verify with unit tests in `config_regen.rs` and `subscriptions` probe-eligibility tests.
