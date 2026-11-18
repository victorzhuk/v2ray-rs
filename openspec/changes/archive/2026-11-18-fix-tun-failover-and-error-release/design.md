# Design

## Context

See `proposal.md` for motivation. Three lifecycle boundaries interact around TUN routing ownership:

1. `crates/process/src/session/mod.rs` parks a failed candidate's `ProcessManager` in `parked` so strict xray TUN routes remain fail-closed across candidate failover. `prepare_tun_session` (`crates/core/src/persistence/tun_session.rs`) intentionally reuses the active `tun_session.v1.json` and `session_id` when the same controller retries on the same backend and interface. Calling `stale.shutdown_with(StopReason::NodeSwitch)` after candidate 2 reaches `Running` invokes `ProcessManager::teardown_tun` against the shared interface and `session_id`, tearing down candidate 2's live tunnel (when `device_ifindex` was `None` or reused) or failing mid-switch.
2. `crates/netctl` replaces priority-8997 exclusion rules on every `xray-up`, while priority-8998 (`--bypass-uid`) and priority-8999 (`--capture-dns`) rules remain in the kernel and journal if an earlier candidate enabled them and a later candidate omits them.
3. `crates/ui/src/app/mod.rs` releases leftover TUN state at the end of `ProcessStateConnection` on terminal `Error`, except when `pending_direct_target`, `deferred_connect`, or `reconnect_after_stop` returns early to start another connection. If that follow-up connection aborts in preflight or runs without TUN, `release_tun_session` is bypassed. Meanwhile `tun_active_for` in `crates/ui/src/app/config_regen.rs` checks only `matches!(state, ProcessState::Running)`, leaving unmarked probes unblocked during `Starting`, `Stopping`, and fail-closed `Error`.

## Goals / Non-Goals

**Goals:**
- Ensure a parked failed candidate never mutates or verifies kernel TUN state that has been adopted by a live replacement candidate.
- Keep `netctl xray-up` idempotent and exact across flag changes within the same session journal.
- Guarantee fail-closed TUN routes are released whenever the UI declines or switches away from starting a TUN session after `Error`.
- Align UI probe gating with actual kernel TUN route presence.

**Non-Goals:**
- Changing the TUN journal file format (`tun_session.v1.json`) or privilege model.
- Adding fwmark bypass plumbing to ephemeral probe backends so probes can run inside active TUN sessions.

## Decisions

1. **Disarm TUN runtime on superseded parked managers before cleanup.**
   When candidate 2 starts with TUN on the same `(backend, iface, session_id)`, candidate 1's child process is already dead (`start_with_connection` failed or crash budget exhausted). Instead of calling `stale.shutdown_with(...)` with `stale.tun` intact after candidate 2 succeeds, clear/disarm `stale`'s `TunRuntime` (or provide an explicit `ProcessManager::abandon_superseded_tun()` helper) once candidate 2 adopts that `session_id`, while keeping full `shutdown_with` teardown when `tun.is_none()` or when backend/iface differs.
   *Alternative considered:* Allocating a new `session_id` per candidate and tearing down candidate 1 before candidate 2 starts. Rejected because tearing down strict routes between candidates opens a plaintext leak window that violates the fail-closed failover contract.

2. **Reconcile 8998 and 8999 rules in `netctl` before applying new optional rules.**
   Extend `Owner` / `net::xray_up` to remove journaled rules at `RULE_PREF_BYPASS_UID` (8998) when `--bypass-uid` is absent (or differs) and at `RULE_PREF_DNS` (8999) when `--capture-dns` is absent, mirroring `remove_exclusions` for 8997.

3. **Centralize leftover-TUN release on aborted/non-TUN follow-up connects in `App`.**
   Add a helper on `App` (e.g., `release_parked_tun_if_idle(&sender)`) invoked on preflight early returns in `AppMsg::Connect`, `AppMsg::ConnectToNode`, and `start_connection` when `matches!(self.process_state, ProcessState::Error(_) | ProcessState::Stopped)` and `self.tun_marker_present()`, and before launching a non-TUN session when `self.tun_marker_present()` is true (or gate launch behind `release_tun_session` completion).

4. **Widen `tun_active_for` to cover all TUN-owned states.**
   Compute `tun_active` from:
   - `matches!(state, ProcessState::Starting | ProcessState::Running | ProcessState::Stopping)` when the active `RuntimeConfigSnapshot` has `tun.enabled && backend_type != BackendType::V2ray`, OR
   - `tun_release_in_flight`, OR
   - `matches!(state, ProcessState::Error(_)) && tun_marker_present`.
   Push updates to `SubscriptionsPage::SetTunActive` whenever any of these inputs changes.

## Risks / Trade-offs

- **[Risk] Launching a non-TUN session while async `release_tun_session` is still running** -> Gate `Connect` / `ConnectToNode` on `tun_release_in_flight` (already present at `app/mod.rs:1344, 1415`) and queue the pending connect as `deferred_connect` until `TunSessionReleased` completes cleanly; if teardown fails, surface the error and keep the non-TUN backend unstarted per `tun-mode` spec.
- **[Risk] Two xray failover session tests and the privileged netctl scenario need a real host** -> They poll `/sys/class/net` and program netlink rules, so the development sandbox cannot execute them; the identity-comparison, abandoned-runtime, sing-box failover, and journal-convergence behaviors are covered by tests that do run, and the gated tests compile with `--features privileged-tests`. CI runs the suite on a host with sysfs, and the netns test needs root.
- **[Risk] `tun_active` staying true after `TunSessionReleased` clears the marker** -> Recompute and forward `SetTunActive` inside the `AppMsg::TunSessionReleased` handler so probes re-enable immediately once routes are gone.
