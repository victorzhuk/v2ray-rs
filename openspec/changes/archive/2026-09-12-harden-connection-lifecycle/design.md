# Design: a closed, controllable crash path

## Context

- `ProcessManager::handle_unexpected_exit` (`crates/process/src/manager.rs:509`) records the crash, calls `teardown_tun` (`netctl xray-down`: delete the policy rules and the device), sleeps `CRASH_RESTART_DELAY × crashes`, then re-runs the full `start_with_connection` — version probe, `getcap`, `xray run -test` (which itself creates and destroys the TUN device), spawn, device wait, `xray-up`. Any start error ends in `Error`.
- The connection task (`crates/ui/src/connection.rs:239`) `select!`s between a Stop command and `wait_and_handle_exit`. Stop drops the wait future mid-respawn and calls `shutdown()`. `stop()` (`manager.rs:261`) returns `Ok` without a transition when there is no child and the state is not `Error`, and `Starting → Stopping` is not an allowed transition (`state.rs:17`).
- A per-manager forwarder (`connection.rs:203`) relays every state except `Error` to the app with the connection's generation. After a give-up the task calls `shutdown()`, whose `Error → Stopped` is relayed; the app treats it as the connection ending (`app.rs:1102`).
- netctl's adds all tolerate `EEXIST` (`crates/netctl/src/net.rs:160-283`), so `xray-up` is idempotent; the device route in table 2023 is device-bound and disappears with the device, after which pref 9002 lookups find an empty table and fall through to `main`.
- `xray_up`/`xray_down` (`crates/process/src/tun.rs:203`) use `.status()` with inherited stdio — the app's stdio is `/dev/null` — and no timeout. `teardown_tun` discards the result.
- `strict_route` is read only by the sing-box generator. `TunRuntime` carries no strictness.
- The TUN marker is written on the GTK thread when `Running` arrives (`app.rs:1128`), after `xray-up` already changed routes, using the interface name from current settings.

## Goals / Non-Goals

**Goals:**
- No traffic leaves outside the tunnel between a crash and the tunnel's return, while the user still intends to be connected (xray, `strict_route` on).
- Every stop request ends in a reported `Stopped`; the app's view of the connection is driven by one source.
- The host is released from the kill-switch exactly when the app stops trying.

**Non-Goals:**
- sing-box crash-window behavior. sing-box owns its `auto_route`/`strict_route` rules; whether they survive a crash or SIGKILL closed is unverified and is its own change.
- Health probing, suspend/resume or network-change handling.
- IPv6 DNS capture without an IPv6 tunnel address (see Risks).
- Changing the crash budget constants or the automatic-reconnect budget.

## Decisions

- **Fail closed with a fallback route, not by holding the device.** `xray-up --strict` adds `unreachable default` with the maximum metric to table 2023 for IPv4, and for IPv6 whenever strict (with or without an IPv6 address). While the device lives, its metric-0 device route wins; when xray dies the kernel removes the device route and the `unreachable` route answers pref 9002 lookups with `EHOSTUNREACH`, so applications fail fast and retry rather than hang or leak. pref 9000 (fwmark 255) and pref 8998 (bypass uid) are ahead of it, so the backend's own dials and bypassed tools keep working; pref 9001 (`main`, `suppress_prefixlength 0`) keeps on-link and LAN routes reachable. Alternative rejected: keeping a persistent TUN device owned by the helper — it needs a long-lived privileged process or `TUNSETPERSIST` plus fd hand-off to xray, which xray only accepts through `XRAY_TUN_FD`, far more moving parts than one route.
- **No teardown between crash and respawn.** Because every `xray-up` step is idempotent and the device route is device-bound, re-running `xray-up` against the new device is safe; the `teardown_tun` call in the respawn path goes. The comment claiming a dirty restart is not idempotent is superseded by a privileged netns test that runs `xray-up`, deletes and recreates the device, and runs `xray-up` again.
- **Release is owned by the app, through the recovery pass.** The manager no longer tears down on give-up either. Failover within a connection task re-runs `xray-up` for the next candidate. When the task ends in `Error` the routes stay; the app keeps them while an automatic reconnect is pending and releases them — by running the same marker-driven route-recovery pass used at startup, then clearing the marker — when it stops trying (budget exhausted), on Disconnect with no live handle, and on Quit. Reusing recovery means release works even for state left by a previous app instance.
- **Terminal states come only from the supervising task.** The forwarder relays `Starting`, `Running` and `Stopping` only. The task emits `Stopped` after any requested stop and `Error` after the last candidate fails. A failover is therefore invisible to the app except as `Starting`/`Running`.
- **`stop()` works from every state.** No child → drive to `Stopped` through `Stopping`; add `Starting → Stopping` to the allowed transitions. With a child, SIGTERM → wait → SIGKILL as today, then teardown. Cancellation safety comes from the helper commands being `kill_on_drop` and from `stop()` inspecting `self.child` rather than trusting the state, not from making the respawn uncancellable — Stop must win over a long respawn.
- **Respawn loop with preflight reuse.** `handle_unexpected_exit` loops: record crash, give up if the window holds `MAX_CRASHES`, sleep, respawn; a respawn error records another crash and continues. The respawn skips the version probe, capability probe and config check: the binary path and config file are the ones that just ran. A binary replaced mid-session that lost its capabilities fails at device creation and spends the budget, which is the correct outcome.
- **Helper calls bounded and heard.** A 10s timeout, `kill_on_drop(true)`, piped stdout/stderr pushed into the log buffer as stderr lines; teardown failures become log lines.
- **Marker from the runtime, before routes.** The connection task writes the marker with `TunRuntime`'s backend and interface before `start_with_connection`, so a crash between `xray-up` and the `Running` message still leaves recovery something to act on.
- **Cancel while starting reuses Disconnect.** The window button and the tray item stay enabled during `Starting` and send the existing Disconnect, which already routes a Stop into the task; no new strings.

## Risks / Trade-offs

- [Strict route blocks the host while reconnecting for up to 3 automatic attempts] → that is the chosen behavior; the final give-up releases it, Disconnect releases it at any time, and the status bar shows Connecting/Error throughout.
- [IPv6 without a tunnel address becomes unreachable] → documented behavior change; the preferences note for `strict_route` states it; users set an IPv6 tunnel address or turn strict off.
- [No IPv6 DNS capture without an IPv6 tunnel address] → a resolver reached over on-link IPv6 still bypasses the tunnel, as today; capturing it would black-hole DNS on IPv6-only upstreams. Left for a follow-up.
- [Skipped preflight on respawn misses a config that became invalid] → the config file is not rewritten during a session; a changed binary fails at spawn and spends the budget.
- [App killed while the kill-switch is up] → the marker survives; the next launch's recovery pass releases it. Until then the host has no default route through the tunnel table — the same state a crashed backend leaves, and exactly what recovery exists for.

## Migration Plan

netctl gains a flag; old helpers ignore nothing because the process crate and the helper ship together. Existing sessions pick it up on the next Connect. Rollback is a revert; a leftover `unreachable` route in table 2023 is removed by `xray-down`/`recover`, which flush the table.

## Implementation plan

Base `7d01aaa`, tier heavy, mode existing-service-strict, estimate 5.7 h. Lenses: spec, quality, sec — spec always; quality for a heavy tier; sec because the change programs host routing through a capability-holding helper.

Rules for whoever executes it: every path is relative to the worktree; `git -C <worktree>` for every git call; one conventional commit per finished task (subject ≤72 chars, imperative, lowercase); never stage `openspec/`; a contract test once written is read-only — changing it is a new task, not an edit; test runs stay bounded (`make test-*` carries `timeout 5m` and 4 threads).

Rust has no separate test-writing stage: in every chunk the tests are the first code tasks, the package's other test files stay unchanged, and the chunk closes when its verify command passes.

### netctl-strict — tasks 1.1, 1.2

- Shape: parallel, shard `netctl`; coder: rust-coder; seam: netctl-strict.
- Files: `crates/netctl/src/main.rs`, `crates/netctl/src/net.rs`, `crates/netctl/tests/privileged.rs`.
- Tests: `privileged::strict_up_installs_fallback_routes_and_v6_rules`, `privileged::down_and_recover_clear_strict_state_both_families`, `privileged::up_down_is_idempotent_in_namespace`, `main::tests::xray_up_parses_strict_flag`.
- Verify: `timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4 && cargo clippy -p v2ray-rs-netctl --all-targets --all-features -- -D warnings`
- Steps:
  - privileged.rs (tests first, own namespaces nctl-strict-ns and nctl-reup-ns, NsGuard, skip-on-no-netns like existing tests): strict_up_installs_fallback_routes_and_v6_rules (1.1: after xray-up --strict without --addr6, `ip route show table 2023` has one line starting 'unreachable default' with 'metric 4294967295'; `ip -6 route show table 2023` same; `ip -6 rule show` has 9000:, 9001:, 9002:, and 8998: when --bypass-uid given; no 8999: in v6); down_and_recover_clear_strict_state_both_families (1.2: after xray-down, and separately after recover --xray, `ip [-6] route show table 2023` empty and `ip [-6] rule show` has none of 8998:..9002:); reup_across_recreated_device_leaves_one_copy (1.3: xray-up --strict, `ip link del nctltest0`, recreate tun, xray-up --strict again; sorted lines of `ip [-6] rule show` and `ip [-6] route show table 2023` equal a fresh single run, each pref exactly once); strict_state_refuses_unmarked_traffic_without_device (1.3: see seeding; v4 `ip -4 route get 198.51.100.1` exits non-zero with 'No route to host'; `ip -4 route get 198.51.100.1 mark 255` succeeds with 'dev nctlmain0'; v6 `ip -6 route get 2001:db8::1` fails or prints 'unreachable'). Add helper ip_in_full(ns,args)->(bool,String) returning stdout+stderr.
  - Extend up_down_is_idempotent_in_namespace: without --strict, table 2023 has no 'unreachable' and `ip -6 rule show` has no 9002: (strict-off preserves previous behavior).
  - main.rs: Command::XrayUp gains `#[arg(long)] strict: bool` with doc line; pass to net::xray_up. XrayDown doc updated to say it also clears table 2023.
  - net.rs: `const FALLBACK_METRIC: u32 = u32::MAX;` fns add_fallback_route_v4(handle) / add_fallback_route_v6(handle): RouteMessageBuilder destination 0/0, table_id(XRAY_ROUTE_TABLE), route type Unreachable, priority FALLBACK_METRIC, EEXIST tolerated. Confirm builder method names (kind/priority) in rtnetlink 0.21 docs before coding.
  - net.rs xray_up signature: `xray_up(handle, iface, v4, v6, bypass_uid, capture_dns, strict: bool)`. When strict: add both fallback routes. v6 policy block runs when `v6.is_some() || strict`: add_xray_rules(Inet6) and bypass-uid v6; add_default_route_v6 and dns capture v6 only when v6.is_some().
  - net.rs xray_down: keep del_xray_rules first, delete TUN device, then flush_table_routes(handle, XRAY_ROUTE_TABLE) (already iterates v4+v6). recover_xray unchanged (calls xray_down then flushes). Update doc comments on xray_up/xray_down.
  - AMENDMENT: new private fns are spelled add_fallback_route_v4 / add_fallback_route_v6 (no add_unreachable_route).
  - AMENDMENT: xray_down flushes table 2023 (flush_table_routes) right after del_xray_rules and before the device delete, so a non-ENODEV link-delete error cannot leave the unreachable routes behind.
  - AMENDMENT: add unprivileged test main.rs tests::xray_up_parses_strict_flag — Cli::try_parse_from(["v2ray-rs-netctl","xray-up","--iface","xtun0","--addr","172.19.0.1/30","--strict"]) yields XrayUp { strict: true, .. }, and without the flag strict is false; this runs under the chunk verify. Privileged tests close only via task 5.2, which blocks archive.
  - AMENDMENT: Command has no Debug derive, so xray_up_parses_strict_flag asserts with matches!(cli.command, Command::XrayUp { strict: true, .. }).
  - Scope: tests for 1.1/1.2 and the strict-off extension only; reup_across_recreated_device_leaves_one_copy and strict_state_refuses_unmarked_traffic_without_device belong to chunk netctl-reup.

### netctl-reup — tasks 1.3

- Shape: after `netctl-strict` (shares crates/netctl); coder: rust-coder; seam: netctl-strict.
- Files: `crates/netctl/tests/privileged.rs`.
- Tests: `privileged::reup_across_recreated_device_leaves_one_copy`, `privileged::strict_state_refuses_unmarked_traffic_without_device`.
- Verify: `timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4 && cargo clippy -p v2ray-rs-netctl --all-targets --all-features -- -D warnings`
- Steps:
  - privileged.rs: add reup_across_recreated_device_leaves_one_copy (xray-up --strict, delete + recreate the tun device, xray-up again → exactly one copy of each table-2023 route and each pref 8998–9002 rule per family) and strict_state_refuses_unmarked_traffic_without_device (strict state, no device: unmarked connect to a public address fails EHOSTUNREACH; a socket with SO_MARK 255 still resolves through main), each in its own namespace with NsGuard and the existing skip-on-no-netns pattern.

### helper-bounded — tasks 2.2, 2.3

- Shape: parallel, shard `integration`; coder: rust-coder; seam: helper-bounded.
- Files: `crates/process/src/tun.rs`, `crates/process/src/manager.rs`, `crates/ui/src/connection.rs`, `crates/process/tests/lifecycle.rs`.
- Tests: `tun::tests::xray_up_args_include_strict_when_set`, `tun::tests::xray_up_args_omit_strict_when_off`, `tun::tests::helper_killed_after_timeout`, `tun::tests::helper_output_is_captured`, `manager::tests::teardown_failure_is_logged`.
- Verify: `make test-process && make test-ui && cargo clippy -p v2ray-rs-process -p v2ray-rs-ui --all-targets --all-features -- -D warnings`
- Steps:
  - tun.rs tests first: xray_up_args_include_strict_when_set, xray_up_args_omit_strict_when_off (exact vec: ['xray-up','--iface','tun0','--addr','172.19.0.1/30'] then optional '--addr6' v6, '--bypass-uid' uid, '--capture-dns', '--strict' in that order); helper_killed_after_timeout (stub script writes $$ to a pid file then `exec sleep 30`; run_helper with 200ms timeout returns Err containing 'timed out', elapsed < 2s, `kill(pid, None)` then fails with ESRCH); helper_output_is_captured (stub `echo up-ok; echo 'netctl: boom' >&2; exit 1` -> output has both lines, result Err containing exit status).
  - manager.rs test: teardown_failure_is_logged (seeding below, helper stub `exit 1`; after stop() state Stopped and log buffer has a line containing 'xray-down failed').
  - tun.rs: `pub strict: bool` on TunRuntime (doc: install the fail-closed fallback routes; xray only). `pub(crate) const HELPER_TIMEOUT: Duration = Duration::from_secs(10);` `pub(crate) fn xray_up_args(rt: &TunRuntime) -> Vec<String>`; `pub(crate) struct HelperRun { pub output: Vec<String>, pub result: Result<(), String> }`; `pub(crate) async fn run_helper(helper: &Path, args: &[String], timeout: Duration) -> HelperRun` (stdin null, stdout/stderr piped, kill_on_drop(true), wait_with_output under tokio::time::timeout; timeout -> result Err('<verb> timed out after 10s')); xray_up(rt) and xray_down(rt) return HelperRun using HELPER_TIMEOUT.
  - manager.rs: `fn log_helper(&self, verb: &str, run: &HelperRun)` pushes each output line as LogLine::stderr into log_buffer and emits ProcessEvent::LogLine (pattern manager.rs:192-199), plus '<verb> failed: <e>' on Err. launch path maps Err to ProcessError::TunHelper(e). teardown_tun logs instead of discarding.
  - connection.rs build_tun_runtime: `strict: settings.tun.strict_route`. Update TunRuntime literals: connection.rs:354, manager.rs:713/799/834, tun.rs:321 (strict: false).
  - AMENDMENT (fixes review blocker; signature unchanged: run_helper(helper: &Path, args: &[String], timeout: Duration) -> HelperRun, ProcessManager::log_helper still does the LogLine push): run_helper spawns with stdin null, stdout/stderr piped and kill_on_drop(true); two spawned reader tasks own the line buffers (outside the timed future), and only child.wait() runs inside tokio::time::timeout; on elapse it calls child.kill().await (SIGKILL + reap, no zombie), then joins the readers bounded by 500ms, and HelperRun carries the captured lines in `output` plus a failure formatted with the actual value `{timeout:?}` (not a hard-coded 10s). Production timeout const HELPER_TIMEOUT = Duration::from_secs(10). tun::tests::helper_killed_after_timeout uses a 200ms timeout and a stub that sleeps 30s, and after Err asserts nix kill(pid, None) == Err(ESRCH) immediately.
  - AMENDMENT: while adding `strict: false` to the TunRuntime literals in manager.rs tests (helper_path PathBuf::from("v2ray-rs-netctl")), switch those helper_path values to a nonexistent absolute path under the test tempdir, so no test can reach the installed helper.

### process-state-stop — tasks 2.1

- Shape: after `helper-bounded` (shares crates/process); coder: rust-coder; seam: process-state-stop.
- Files: `crates/process/src/state.rs`, `crates/process/src/manager.rs`, `crates/process/tests/lifecycle.rs`.
- Tests: `state::tests::starting_can_move_to_stopping`, `manager::tests::stop_from_starting_without_child_reaches_stopped`, `manager::tests::stop_from_running_without_child_reaches_stopped`, `manager::tests::stop_from_error_releases_tun_state`.
- Verify: `make test-process && cargo clippy -p v2ray-rs-process --all-targets --all-features -- -D warnings`
- Steps:
  - Tests first. state.rs: starting_can_move_to_stopping. manager.rs: stop_from_starting_without_child_reaches_stopped, stop_from_running_without_child_reaches_stopped, stop_from_error_releases_tun_state (tun stub attached, helper calls file has exactly one 'xray-down'); each asserts subscribe() yields StateChanged to Stopping then Stopped (Error row: Stopped only) and stop() returns Ok.
  - state.rs can_transition_to: add (Starting, Stopping).
  - manager.rs stop(): child None and Stopped -> return Ok, no event. child None and Error -> teardown_tun, Error->Stopped. Otherwise: transition to Stopping unless already Stopping; graceful_stop (no-op without child); teardown_tun; Stopped; pid_file.remove. Remove the old early return at manager.rs:262-270.
  - Keep shutdown() = auto_restart false + stop(). Update existing test stop_recovers_from_error_without_child only if its assertions change (they should not).

### respawn-loop — tasks 2.4, 2.5

- Shape: after `process-state-stop` (shares crates/process); coder: rust-coder; seam: respawn-loop.
- Files: `crates/process/src/manager.rs`, `crates/process/tests/lifecycle.rs`.
- Tests: `manager::tests::respawn_skips_preflight`, `manager::tests::failed_respawn_is_retried`, `manager::tests::respawn_budget_exhaustion_errors`, `manager::tests::stop_during_respawn_wait_reaches_stopped`.
- Verify: `make test-process && cargo clippy -p v2ray-rs-process --all-targets --all-features -- -D warnings`
- Steps:
  - Tests first (manager.rs, restart_delay 50ms): respawn_skips_preflight (SingBox check stub logs 'check' to a file; backend run 1 exits 1, run 2 sleeps; after wait_and_handle_exit: Running, checks=1, runs=2); failed_respawn_is_retried (tun stub on 'lo', helper fails first xray-up and succeeds second; ends Running; calls = xray-up,xray-up; no xray-down); respawn_budget_exhaustion_errors (backend always exit 1 after first spawn; loop `while mgr.state() == Running { mgr.wait_and_handle_exit().await }`; Error containing '3 crashes'; calls has zero 'xray-down'); stop_during_respawn_wait_reaches_stopped (restart_delay 5s; timeout(300ms, wait_and_handle_exit) then shutdown(); Stopped, calls exactly one 'xray-down', zero 'xray-up').
  - manager.rs: field `restart_delay: Duration` initialized to CRASH_RESTART_DELAY in new().
  - Rename spawn_process -> `async fn launch(&mut self) -> Result<(), ProcessError>`: on device timeout or xray-up failure run graceful_stop only, no teardown_tun (drop manager.rs:343,350,355 teardown calls).
  - Add `async fn respawn(&mut self) -> Result<(), ProcessError>`: `self.launch().await?` then Starting->Running with current_connection. Reuses binary_path, config_path, geodata_dir, tun, current_connection unchanged since the start; skips xray_version_triple, has_net_admin, check_config.
  - handle_unexpected_exit: record crash; if !auto_restart -> Error(msg), return. Loop: if crash_times.len() >= MAX_CRASHES -> Error('{MAX_CRASHES} crashes within {CRASH_WINDOW:?}: {msg}'), return; if state is Running -> transition Starting(current_connection) (never Starting->Starting); sleep(restart_delay * crashes); respawn(): Ok -> return; Err(e) -> log 'restart failed: {e}', msg = that, record crash, continue. Remove the pre-respawn teardown_tun and its comment (manager.rs:527-530).
  - AMENDMENT: respawn_skips_preflight also asserts a fresh ProcessManager::new(..) has restart delay == CRASH_RESTART_DELAY (the 2s spec default), since the other respawn tests override it to 50ms.

### conn-terminal-state — tasks 3.1, 3.2

- Shape: after `respawn-loop` (shares crates/ui); coder: rust-coder; seam: conn-terminal-state.
- Files: `crates/ui/src/connection.rs`, `crates/ui/src/app.rs`, `crates/core/src/persistence/tun_session.rs`.
- Tests: `connection::tests::forwarder_relays_only_nonterminal_states`, `connection::tests::failover_reports_no_stopped_and_stop_reports_one`, `connection::tests::last_candidate_failure_reports_one_error`, `connection::tests::marker_written_from_runtime_before_start`.
- Verify: `make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings`
- Steps:
  - Tests first (connection.rs, #[tokio::test(flavor = 'multi_thread')]): forwarder_relays_only_nonterminal_states (pure); failover_reports_no_stopped_and_stop_reports_one; last_candidate_failure_reports_one_error; marker_written_from_runtime_before_start. No manager trait: the seam is connection::spawn with a stub backend script and `relm4::channel::<AppMsg>()` (re-exported by relm4 0.10; confirm) as the sender.
  - `fn relays(state: &ProcessState) -> bool` = Starting|Running|Stopping; forwarder uses it; keep its JoinHandle.
  - Before every terminal emit and before failing over: `forwarder.abort(); let _ = forwarder.await;` so no relayed state follows the terminal one.
  - Failover and start-failure paths drop shutdown(): keep the failed manager as `parked: Option<ProcessManager>` (replaced per candidate, dropped at final Error). Stop at loop top or pre-start: `if let Some(mut m) = parked.take() { m.shutdown().await }` then emit Stopped, so leftover routes are released.
  - Wrap start_with_connection in `tokio::select! { biased; Some(ConnectionCmd::Stop) = cmd_rx.recv() => { mgr.shutdown().await; emit Stopped; return } r = mgr.start_with_connection(..) => r }`. Supervise loop Stop branch: shutdown, abort forwarder, emit Stopped, return.
  - ConnectionRequest gains `pub paths: AppPaths`; app.rs start_connection passes self.paths.clone(). `fn tun_session_for(rt: &TunRuntime) -> TunSession`. Per candidate, when the runtime is Some, save_tun_session before start_with_connection; failure -> log::warn. Delete the marker write at app.rs:1128-1142.
  - AMENDMENT: marker_written_from_runtime_before_start uses a single candidate and a stub that answers `version` at once (`[ "$1" = version ] && echo "Xray 26.6.27" && exit 0`), so it never waits CONFIG_CHECK_TIMEOUT.
  - AMENDMENT: parking a failed manager aborts its log-forwarder task together with its state forwarder, so a failed candidate cannot emit log lines after the next candidate starts.

### app-killswitch — tasks 3.3, 3.4

- Shape: after `conn-terminal-state` (shares crates/ui); coder: rust-coder; seam: app-killswitch.
- Files: `crates/ui/src/app.rs`.
- Tests: `app::tests::quit_while_stopping_waits_for_stopped`, `app::tests::quit_with_handle_stops_first`, `app::tests::quit_with_leftover_marker_releases_first`, `app::tests::quit_idle_exits`, `app::tests::auto_reconnect_exhausted_after_three_attempts`, `app::tests::auto_reconnect_suppressed_on_exit`, `app::tests::recover_runs_helper_and_clears_marker`, `app::tests::recover_clears_marker_when_helper_fails`, `app::tests::disconnect_without_handle_releases_marker`, `app::tests::error_while_stopping_releases_without_retry`, `app::tests::error_with_reconnects_left_keeps_killswitch`.
- Verify: `make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings`
- Steps:
  - Tests first (app.rs pure fns): quit_while_stopping_waits_for_stopped, quit_with_handle_stops_first, quit_with_leftover_marker_releases_first, quit_idle_exits, auto_reconnect_exhausted_after_three_attempts, auto_reconnect_suppressed_on_exit, recover_runs_helper_and_clears_marker (stub helper writes "$@"; args exactly 'recover --xray --iface tun9'), recover_clears_marker_when_helper_fails (stub exit 1).
  - `recover_tun_session(paths: &AppPaths, helper: &Path)`; init caller passes &v2ray_rs_process::helper_path(). Tests never run the real helper.
  - `enum QuitPlan { Stop, AwaitStopped, Release, Exit }`, `fn quit_plan(has_handle: bool, state: &ProcessState, marker_present: bool) -> QuitPlan`; TrayQuit and CloseRequested (non-tray branch) use it; AwaitStopped and Release set pending_exit.
  - `fn auto_reconnect_allowed(pending_exit: bool, attempts: u32) -> bool`; schedule_auto_reconnect returns bool.
  - `fn release_tun_session(&mut self, sender)`: sets `tun_release_in_flight`, tokio::spawn { lifecycle.lock(); spawn_blocking(recover_tun_session) } then `AppMsg::TunReleased`. Handler clears the flag; destroys window if pending_exit. Connect and ConnectToNode return early while the flag is set.
  - ProcessStateConnection Error: release when no retry follows (no reconnect_pending, no replay target, auto_reconnect_allowed false or not scheduled, direct_connect_in_flight, pending_exit, or app state was Stopping). Disconnect without handle and marker present: cancel_auto_reconnect, apply_state(Stopped), release; else existing 'Not connected' toast.
  - AMENDMENT: extract pure fns beside quit_plan: disconnect_plan(has_handle: bool, marker_present: bool) -> DisconnectPlan { Stop, Release, Nothing } and release_on_error(app_state_stopping: bool, reconnects_left: u32) -> bool — the retry-budget sub-decision only; the Error handler still combines it with reconnect_pending, the replay target, direct_connect_in_flight and pending_exit as today. The Stopped report after a Release rests on apply_state(Stopped) in the handler (accepted gap, no handler-level test). Tests app::tests::disconnect_without_handle_releases_marker and app::tests::error_while_stopping_releases_without_retry, plus app::tests::error_with_reconnects_left_keeps_killswitch.

### cancel-starting — tasks 3.5

- Shape: after `app-killswitch` (shares crates/ui); coder: rust-coder; seam: cancel-starting.
- Files: `crates/ui/src/app.rs`, `crates/tray/src/tray.rs`.
- Tests: `app::tests::toggle_is_actionable_disconnect_while_starting`, `app::tests::toggle_disabled_while_stopping`, `tray::tests::toggle_is_enabled_disconnect_while_starting`, `tray::tests::toggle_disabled_while_stopping`.
- Verify: `make test-ui && make test-tray && cargo clippy -p v2ray-rs-ui -p v2ray-rs-tray --all-targets --all-features -- -D warnings`
- Steps:
  - Tests first: app::tests::toggle_is_actionable_disconnect_while_starting, app::tests::toggle_disabled_while_stopping, tray::tests::toggle_is_enabled_disconnect_while_starting, tray::tests::toggle_disabled_while_stopping (TrayAction has no PartialEq: use matches!).
  - app.rs `fn connect_toggle(state: &ProcessState) -> (bool, bool)` = (shows_disconnect, sensitive): Stopped (false,true), Starting (true,true), Running (true,true), Stopping (true,false), Error (false,true); apply_state sets connected/button_sensitive from it. `connected` is read only by the button and ToggleConnection.
  - tray.rs `fn toggle_action(state: &ProcessState) -> (TrayAction, bool)`: Running|Starting (Disconnect,true), Stopping (Connect,false), Stopped|Error (Connect,true); menu() builds the item from it.

### prefs-docs — tasks 4.1, 4.2

- Shape: after `cancel-starting` (shares crates/ui); coder: rust-coder; seam: prefs-docs.
- Files: `crates/ui/src/preferences/tun.rs`, `docs/ARCHITECTURE.md`, `CHANGELOG.md`.
- Tests: `preferences::tun::tests::strict_route_row_sensitive_for_xray`.
- Verify: `make test-ui && rg -n strict_route docs/ARCHITECTURE.md CHANGELOG.md`
- Steps:
  - preferences/tun.rs new test module: strict_route_row_sensitive_for_xray (strict_route_applies(Xray) and (SingBox) true, (V2ray) false).
  - `fn strict_route_applies(backend: BackendType) -> bool`; strict_row.set_sensitive uses it at tun.rs:725 and :736; strict row subtitle states it applies to both backends and, under xray, blocks traffic while reconnecting and IPv6 without an IPv6 tunnel address; advanced_note (tun.rs:126-130) narrowed so it no longer claims strict is sing-box only.
  - docs/ARCHITECTURE.md: state machine (104-106: Starting->Stopping), crash recovery (108-118: respawn loop, preflight reuse, no teardown), netctl (148-165: --strict, v6 rules, xray-down clears table 2023), stop/marker (229-238: teardown only from stop, release owned by the app, marker written by the connection task before start).
  - CHANGELOG.md [Unreleased] Changed: BREAKING for xray TUN with strict_route on (default): IPv6 refused without an IPv6 tunnel address; host blocked while reconnecting. Fixed: Disconnect during respawn/start, failover handle loss, helper timeout.

### verify-floor — tasks 5.1, 5.2

- Shape: after `prefs-docs` (shares workspace); coder: zpatcher; seam: verification.
- Files: `Makefile`, `crates/netctl/tests/privileged.rs`.
- Verify: `make lint && make test TEST_TIMEOUT=10m`

### verify-live — tasks 5.3, 5.4

- Shape: after `verify-floor` (shares workspace); coder: zpatcher; seam: verification.
- Files: `crates/netctl/src/net.rs`.
- Verify: `manual: live xray TUN host checks 5.3/5.4, run by the user after merge`

### Waivers

- netctl-strict: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- helper-bounded: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- process-state-stop: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- respawn-loop: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- conn-terminal-state: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- app-killswitch: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- cancel-starting: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- prefs-docs: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- verification: NO-RED-WAIVER: manual live check; NO-TESTER-WAIVER: manual live check, run by the user after merge.

### Floor

`make lint && make test TEST_TIMEOUT=10m`

Tasks 5.2 (root netns suite: `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' timeout 5m cargo test -p v2ray-rs-netctl --features privileged-tests -- --test-threads=2`), 5.3 and 5.4 (live xray host) are run by hand after merge; the change is not archived until they are reported.

### Risks

- Sibling changes fix-connection-ui-state and persist-backend-diagnostics touch app.rs, connection.rs and manager.rs -> planned against HEAD 7d01aaa; land this first or rebase them; conflict hotspots app.rs ProcessStateConnection handler and connection.rs spawn.
- strict_route defaults true (core/models/tun.rs:61-74) -> every xray TUN user gets the kill-switch and the IPv6 block on upgrade; mitigated by the CHANGELOG BREAKING entry and the prefs note.
- Forwarder races the task's own terminal emit -> abort and join the forwarder before every terminal emit and failover.
- Release vs a fresh Connect both take the lifecycle lock, order not guaranteed -> tun_release_in_flight gates Connect until TunReleased.
- Error racing a user Disconnect used to schedule an auto-reconnect -> Error while app state Stopping releases and never retries.
- Reconnect under a held kill-switch: pin_node_addresses lookups to off-link resolvers fail fast -> hosts not pinned, capture_dns off for that session; on-link resolvers still work via pref 9001; follow-up.
- Connection tests spend about 6s per crash-looping candidate on real restart delays -> accepted instead of a public test-only builder; bounded under 30s per test.
- A test TunRuntime whose helper resolves to the installed netctl could flush the dev host's table 2023 -> every helper-reaching test uses a tempdir stub; listed as forbidden.

### Plan review

Pass after two rounds by an independent reviewer. Round 1 blocked on a helper-timeout test that probed an unreaped child; the helper now kills and reaps on timeout. The remaining findings became the AMENDMENT steps above. Accepted gaps: the privileged netns tests run only under root (task 5.2, which blocks archive), and the `Stopped` report after a Disconnect with no live handle is covered by the handler, not by a test.

## Plan appendix

```json
{
  "v": 2,
  "change": "harden-connection-lifecycle",
  "baseSha": "7d01aaa42720c63f149aa66330f25a82db6fa694",
  "generatedAt": "2026-09-11T16:53:15.645Z",
  "tier": "heavy",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality",
    "sec"
  ],
  "estimateHours": 5.7,
  "chunks": [
    {
      "id": "netctl-strict",
      "taskIds": [
        "1.1",
        "1.2"
      ],
      "seam": "netctl-strict",
      "contract": {
        "states": [
          "Clean",
          "Up",
          "UpStrict",
          "DeviceGone"
        ],
        "transitions": [
          {
            "input": "xray-up without --strict, without --addr6",
            "state": "Up",
            "effect": "set",
            "evidence": "net.rs:61-105; tun-mode 'Strict route off keeps previous behavior': v4 device route + v4 9000/9001/9002 only, no unreachable, no v6 rules"
          },
          {
            "input": "xray-up --strict without --addr6",
            "state": "UpStrict",
            "effect": "set",
            "evidence": "tun-mode 'Strict fallback routes': unreachable default metric 4294967295 table 2023 v4+v6; v6 9000/9001/9002 (+8998 with --bypass-uid); no v6 8999, no v6 device route"
          },
          {
            "input": "xray-up --strict --addr6",
            "state": "UpStrict",
            "effect": "set",
            "evidence": "net.rs:81-102 plus fallback routes; v6 8999 only with --capture-dns"
          },
          {
            "input": "TUN device deleted while UpStrict",
            "state": "DeviceGone",
            "effect": "forced",
            "evidence": "design 'Fail closed with a fallback route': kernel drops device routes, unreachable answers pref 9002; pref 9000 mark 255 and 8998 still reach main"
          },
          {
            "input": "xray-up --strict on recreated device",
            "state": "UpStrict",
            "effect": "no-op",
            "evidence": "EEXIST tolerance net.rs:160-283; tun-mode 'Re-running up across a recreated device'"
          },
          {
            "input": "xray-down from any state",
            "state": "Clean",
            "effect": "clear",
            "evidence": "tun-mode 'Tear xray TUN routes down': prefs 8998-9002 both families, table 2023 flushed both families, TUN device deleted; no-op when absent"
          },
          {
            "input": "recover --xray from any state",
            "state": "Clean",
            "effect": "clear",
            "evidence": "net.rs:132-136; tun-mode 'Recover leftovers'"
          },
          {
            "input": "xray-up on non-TUN iface",
            "state": "Clean",
            "effect": "no-op",
            "evidence": "main.rs:82 refuses before any netlink call"
          }
        ],
        "forbidden": [
          "unreachable route in table 2023 after xray-up without --strict",
          "v6 pref 8999 rule without --addr6",
          "fallback metric lower than or equal to the device route metric",
          "deleting rules outside prefs 8998-9002 or routes outside table 2023"
        ],
        "seeding": [
          "All states only inside a throwaway netns via existing helpers netctl_in/ip_in (privileged.rs:28-63); device = `ip tuntap add dev nctltest0 mode tun`",
          "DeviceGone: in nctl-strict-ns add second tun nctlmain0, `ip addr add 10.99.0.1/24 dev nctlmain0`, `ip link set nctlmain0 up`, `ip route add default dev nctlmain0` (main default), xray-up --strict on nctltest0, then `ip link del nctltest0`"
        ],
        "budgets": [
          "FALLBACK_METRIC = u32::MAX = 4294967295",
          "privileged suite wall-clock 5m, --test-threads=2",
          "netlink ops unbounded inside netctl; the caller bounds the helper at 10s (helper-bounded)"
        ],
        "names": [
          "--strict",
          "Command::XrayUp.strict",
          "FALLBACK_METRIC",
          "add_fallback_route_v4",
          "add_fallback_route_v6",
          "net::xray_up(.., strict: bool)",
          "privileged::strict_up_installs_fallback_routes_and_v6_rules",
          "privileged::down_and_recover_clear_strict_state_both_families",
          "privileged::reup_across_recreated_device_leaves_one_copy",
          "privileged::strict_state_refuses_unmarked_traffic_without_device"
        ],
        "refusals": [
          "non-TUN --iface: netctl main.rs before netlink, exit 1 'refusing xray-up on <iface>: not a TUN device'",
          "bad CIDR: validate::parse_cidr before netlink"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "privileged.rs (tests first, own namespaces nctl-strict-ns and nctl-reup-ns, NsGuard, skip-on-no-netns like existing tests): strict_up_installs_fallback_routes_and_v6_rules (1.1: after xray-up --strict without --addr6, `ip route show table 2023` has one line starting 'unreachable default' with 'metric 4294967295'; `ip -6 route show table 2023` same; `ip -6 rule show` has 9000:, 9001:, 9002:, and 8998: when --bypass-uid given; no 8999: in v6); down_and_recover_clear_strict_state_both_families (1.2: after xray-down, and separately after recover --xray, `ip [-6] route show table 2023` empty and `ip [-6] rule show` has none of 8998:..9002:); reup_across_recreated_device_leaves_one_copy (1.3: xray-up --strict, `ip link del nctltest0`, recreate tun, xray-up --strict again; sorted lines of `ip [-6] rule show` and `ip [-6] route show table 2023` equal a fresh single run, each pref exactly once); strict_state_refuses_unmarked_traffic_without_device (1.3: see seeding; v4 `ip -4 route get 198.51.100.1` exits non-zero with 'No route to host'; `ip -4 route get 198.51.100.1 mark 255` succeeds with 'dev nctlmain0'; v6 `ip -6 route get 2001:db8::1` fails or prints 'unreachable'). Add helper ip_in_full(ns,args)->(bool,String) returning stdout+stderr.",
        "Extend up_down_is_idempotent_in_namespace: without --strict, table 2023 has no 'unreachable' and `ip -6 rule show` has no 9002: (strict-off preserves previous behavior).",
        "main.rs: Command::XrayUp gains `#[arg(long)] strict: bool` with doc line; pass to net::xray_up. XrayDown doc updated to say it also clears table 2023.",
        "net.rs: `const FALLBACK_METRIC: u32 = u32::MAX;` fns add_fallback_route_v4(handle) / add_fallback_route_v6(handle): RouteMessageBuilder destination 0/0, table_id(XRAY_ROUTE_TABLE), route type Unreachable, priority FALLBACK_METRIC, EEXIST tolerated. Confirm builder method names (kind/priority) in rtnetlink 0.21 docs before coding.",
        "net.rs xray_up signature: `xray_up(handle, iface, v4, v6, bypass_uid, capture_dns, strict: bool)`. When strict: add both fallback routes. v6 policy block runs when `v6.is_some() || strict`: add_xray_rules(Inet6) and bypass-uid v6; add_default_route_v6 and dns capture v6 only when v6.is_some().",
        "net.rs xray_down: keep del_xray_rules first, delete TUN device, then flush_table_routes(handle, XRAY_ROUTE_TABLE) (already iterates v4+v6). recover_xray unchanged (calls xray_down then flushes). Update doc comments on xray_up/xray_down.",
        "AMENDMENT: new private fns are spelled add_fallback_route_v4 / add_fallback_route_v6 (no add_unreachable_route).",
        "AMENDMENT: xray_down flushes table 2023 (flush_table_routes) right after del_xray_rules and before the device delete, so a non-ENODEV link-delete error cannot leave the unreachable routes behind.",
        "AMENDMENT: add unprivileged test main.rs tests::xray_up_parses_strict_flag — Cli::try_parse_from([\"v2ray-rs-netctl\",\"xray-up\",\"--iface\",\"xtun0\",\"--addr\",\"172.19.0.1/30\",\"--strict\"]) yields XrayUp { strict: true, .. }, and without the flag strict is false; this runs under the chunk verify. Privileged tests close only via task 5.2, which blocks archive.",
        "AMENDMENT: Command has no Debug derive, so xray_up_parses_strict_flag asserts with matches!(cli.command, Command::XrayUp { strict: true, .. }).",
        "Scope: tests for 1.1/1.2 and the strict-off extension only; reup_across_recreated_device_leaves_one_copy and strict_state_refuses_unmarked_traffic_without_device belong to chunk netctl-reup."
      ],
      "coder": "rust-coder",
      "verify": "timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4 && cargo clippy -p v2ray-rs-netctl --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "1.1",
          "file": "crates/netctl/src/main.rs",
          "symbol": "Command::XrayUp",
          "anchor": "capture_dns: bool,",
          "lines": "21-34",
          "change": "add `#[arg(long)] strict: bool` flag to xray-up"
        },
        {
          "task": "1.1",
          "file": "crates/netctl/src/main.rs",
          "symbol": "run (XrayUp arm)",
          "anchor": "net::xray_up(&handle, &iface, v4, v6, bypass_uid, capture_dns).await",
          "lines": "71-89",
          "change": "destructure `strict` and pass it to net::xray_up"
        },
        {
          "task": "1.1",
          "file": "crates/netctl/src/net.rs",
          "symbol": "xray_up",
          "anchor": "pub async fn xray_up(",
          "lines": "61-105",
          "change": "take `strict`; when set add unreachable default (max metric) to table 2023 for v4, and for v6 install add_xray_rules(Inet6) + v6 unreachable default even when v6 is None (dns-capture/bypass-uid v6 rules follow the same gate)"
        },
        {
          "task": "1.1",
          "file": "crates/netctl/src/net.rs",
          "symbol": "add_fallback_route_v4 / add_fallback_route_v6 (NEW)",
          "anchor": "NEW",
          "change": "NEW fn: per-family `unreachable default` in XRAY_ROUTE_TABLE with max priority/metric, EEXIST tolerated via is_exists; place beside add_default_route_v4/v6 (net.rs:172-198)"
        },
        {
          "task": "1.1",
          "file": "crates/netctl/src/net.rs",
          "symbol": "rule/table constants",
          "anchor": "const RULE_PREF_TUN: u32 = 9002;",
          "lines": "19-47",
          "change": "reference: XRAY_ROUTE_TABLE=2023 (19), prefs 8998 BYPASS_UID/8999 DNS/9000 BYPASS/9001 MAIN/9002 TUN (30-44), EEXIST=-17 (46); add a max-metric const for the fallback route (NEW)"
        },
        {
          "task": "1.1",
          "file": "crates/netctl/src/net.rs",
          "symbol": "is_exists",
          "anchor": "fn is_exists(err: &rtnetlink::Error) -> bool",
          "lines": "390-392",
          "change": "reuse for EEXIST tolerance of the new route adds; no change"
        },
        {
          "task": "1.1",
          "file": "crates/netctl/tests/privileged.rs",
          "symbol": "strict fallback test (NEW)",
          "anchor": "const NS_DNS: &str = \"nctl-dns-ns\";",
          "lines": "11-17",
          "change": "NEW test with own netns const: `xray-up --strict` (no --addr6) → `ip route show table 2023` / `ip -6 route show table 2023` contain `unreachable default`, `ip -6 rule show` contains 9000/9001/9002"
        },
        {
          "task": "1.2",
          "file": "crates/netctl/src/net.rs",
          "symbol": "xray_down",
          "anchor": "pub async fn xray_down(handle: &Handle, iface: &str) -> Result<(), String> {",
          "lines": "110-128",
          "change": "also remove the unreachable fallback routes from table 2023 (both families), e.g. flush_table_routes(XRAY_ROUTE_TABLE); rules already cleared for V4+V6 by del_xray_rules"
        },
        {
          "task": "1.2",
          "file": "crates/netctl/src/net.rs",
          "symbol": "recover_xray",
          "anchor": "pub async fn recover_xray(handle: &Handle, iface: &str) -> Result<(), String> {",
          "lines": "132-136",
          "change": "already flushes table 2023; confirm the v4/v6 route dump in flush_table_routes returns unreachable-type routes, extend if not"
        },
        {
          "task": "1.2",
          "file": "crates/netctl/src/net.rs",
          "symbol": "flush_table_routes",
          "anchor": "async fn flush_table_routes(handle: &Handle, table: u32) {",
          "lines": "359-378",
          "change": "dumps v4+v6 via RouteMessageBuilder::new().build() and deletes by table; verify it matches RTN_UNREACHABLE routes"
        },
        {
          "task": "1.2",
          "file": "crates/netctl/src/net.rs",
          "symbol": "del_xray_rules / is_xray_rule",
          "anchor": "async fn del_xray_rules(handle: &Handle) {",
          "lines": "320-346",
          "change": "iterates IpVersion::V4+V6, deletes prefs 8998-9002; no change expected, covered by new test"
        },
        {
          "task": "1.2",
          "file": "crates/netctl/tests/privileged.rs",
          "symbol": "strict teardown test (NEW)",
          "anchor": "NEW",
          "change": "NEW netns test: after `xray-down` and after `recover --xray` of a strict session, `ip [-6] route show table 2023` empty and `ip [-6] rule show` has no 8998:-9002:"
        }
      ],
      "pkgDirs": [
        "crates/netctl"
      ],
      "pkgs": [
        "v2ray-rs-netctl"
      ],
      "parallel": true,
      "shard": "netctl",
      "targetedTests": [
        "privileged::strict_up_installs_fallback_routes_and_v6_rules",
        "privileged::down_and_recover_clear_strict_state_both_families",
        "privileged::up_down_is_idempotent_in_namespace",
        "main::tests::xray_up_parses_strict_flag"
      ]
    },
    {
      "id": "netctl-reup",
      "taskIds": [
        "1.3"
      ],
      "seam": "netctl-strict",
      "contract": {
        "states": [
          "Clean",
          "Up",
          "UpStrict",
          "DeviceGone"
        ],
        "transitions": [
          {
            "input": "xray-up without --strict, without --addr6",
            "state": "Up",
            "effect": "set",
            "evidence": "net.rs:61-105; tun-mode 'Strict route off keeps previous behavior': v4 device route + v4 9000/9001/9002 only, no unreachable, no v6 rules"
          },
          {
            "input": "xray-up --strict without --addr6",
            "state": "UpStrict",
            "effect": "set",
            "evidence": "tun-mode 'Strict fallback routes': unreachable default metric 4294967295 table 2023 v4+v6; v6 9000/9001/9002 (+8998 with --bypass-uid); no v6 8999, no v6 device route"
          },
          {
            "input": "xray-up --strict --addr6",
            "state": "UpStrict",
            "effect": "set",
            "evidence": "net.rs:81-102 plus fallback routes; v6 8999 only with --capture-dns"
          },
          {
            "input": "TUN device deleted while UpStrict",
            "state": "DeviceGone",
            "effect": "forced",
            "evidence": "design 'Fail closed with a fallback route': kernel drops device routes, unreachable answers pref 9002; pref 9000 mark 255 and 8998 still reach main"
          },
          {
            "input": "xray-up --strict on recreated device",
            "state": "UpStrict",
            "effect": "no-op",
            "evidence": "EEXIST tolerance net.rs:160-283; tun-mode 'Re-running up across a recreated device'"
          },
          {
            "input": "xray-down from any state",
            "state": "Clean",
            "effect": "clear",
            "evidence": "tun-mode 'Tear xray TUN routes down': prefs 8998-9002 both families, table 2023 flushed both families, TUN device deleted; no-op when absent"
          },
          {
            "input": "recover --xray from any state",
            "state": "Clean",
            "effect": "clear",
            "evidence": "net.rs:132-136; tun-mode 'Recover leftovers'"
          },
          {
            "input": "xray-up on non-TUN iface",
            "state": "Clean",
            "effect": "no-op",
            "evidence": "main.rs:82 refuses before any netlink call"
          }
        ],
        "forbidden": [
          "unreachable route in table 2023 after xray-up without --strict",
          "v6 pref 8999 rule without --addr6",
          "fallback metric lower than or equal to the device route metric",
          "deleting rules outside prefs 8998-9002 or routes outside table 2023"
        ],
        "seeding": [
          "All states only inside a throwaway netns via existing helpers netctl_in/ip_in (privileged.rs:28-63); device = `ip tuntap add dev nctltest0 mode tun`",
          "DeviceGone: in nctl-strict-ns add second tun nctlmain0, `ip addr add 10.99.0.1/24 dev nctlmain0`, `ip link set nctlmain0 up`, `ip route add default dev nctlmain0` (main default), xray-up --strict on nctltest0, then `ip link del nctltest0`"
        ],
        "budgets": [
          "FALLBACK_METRIC = u32::MAX = 4294967295",
          "privileged suite wall-clock 5m, --test-threads=2",
          "netlink ops unbounded inside netctl; the caller bounds the helper at 10s (helper-bounded)"
        ],
        "names": [
          "--strict",
          "Command::XrayUp.strict",
          "FALLBACK_METRIC",
          "add_fallback_route_v4",
          "add_fallback_route_v6",
          "net::xray_up(.., strict: bool)",
          "privileged::strict_up_installs_fallback_routes_and_v6_rules",
          "privileged::down_and_recover_clear_strict_state_both_families",
          "privileged::reup_across_recreated_device_leaves_one_copy",
          "privileged::strict_state_refuses_unmarked_traffic_without_device"
        ],
        "refusals": [
          "non-TUN --iface: netctl main.rs before netlink, exit 1 'refusing xray-up on <iface>: not a TUN device'",
          "bad CIDR: validate::parse_cidr before netlink"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "privileged.rs: add reup_across_recreated_device_leaves_one_copy (xray-up --strict, delete + recreate the tun device, xray-up again → exactly one copy of each table-2023 route and each pref 8998–9002 rule per family) and strict_state_refuses_unmarked_traffic_without_device (strict state, no device: unmarked connect to a public address fails EHOSTUNREACH; a socket with SO_MARK 255 still resolves through main), each in its own namespace with NsGuard and the existing skip-on-no-netns pattern."
      ],
      "coder": "rust-coder",
      "verify": "timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4 && cargo clippy -p v2ray-rs-netctl --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "1.3",
          "file": "crates/netctl/tests/privileged.rs",
          "symbol": "recreate idempotency + EHOSTUNREACH tests (NEW)",
          "anchor": "fn up_down_is_idempotent_in_namespace() {",
          "lines": "79-201",
          "change": "NEW tests beside existing: xray-up, `ip tuntap del`/add, xray-up → exactly one of each route/rule (count lines); strict with no device → unmarked connect to public addr fails EHOSTUNREACH, fwmark-255 socket reaches main"
        }
      ],
      "pkgDirs": [
        "crates/netctl"
      ],
      "pkgs": [
        "v2ray-rs-netctl"
      ],
      "parallel": false,
      "shard": "netctl",
      "prev": "netctl-strict",
      "sharedPkg": "crates/netctl",
      "targetedTests": [
        "privileged::reup_across_recreated_device_leaves_one_copy",
        "privileged::strict_state_refuses_unmarked_traffic_without_device"
      ]
    },
    {
      "id": "helper-bounded",
      "taskIds": [
        "2.2",
        "2.3"
      ],
      "seam": "helper-bounded",
      "contract": {
        "states": [
          "HelperRunning",
          "HelperOk",
          "HelperFailed",
          "HelperTimedOut",
          "HelperCancelled"
        ],
        "transitions": [
          {
            "input": "helper exits 0",
            "state": "HelperOk",
            "effect": "set",
            "evidence": "tasks 2.2; output lines reach log stream as stderr lines"
          },
          {
            "input": "helper exits non-zero",
            "state": "HelperFailed",
            "effect": "set",
            "evidence": "tun-mode 'Route helper output is visible'; xray-up -> ProcessError::TunHelper"
          },
          {
            "input": "helper still running at HELPER_TIMEOUT",
            "state": "HelperTimedOut",
            "effect": "forced",
            "evidence": "tun-mode 'Route helper hangs': child killed, treated as failed, logged"
          },
          {
            "input": "calling future dropped (Stop wins)",
            "state": "HelperCancelled",
            "effect": "forced",
            "evidence": "design 'Cancellation safety comes from kill_on_drop'"
          },
          {
            "input": "helper spawn error (ENOENT/EACCES)",
            "state": "HelperFailed",
            "effect": "set",
            "evidence": "io error string in result, logged"
          },
          {
            "input": "xray-down fails inside stop()",
            "state": "HelperFailed",
            "effect": "no-op",
            "evidence": "tasks 2.2 'teardown_tun logs failures'; stop still reaches Stopped"
          },
          {
            "input": "TunRuntime.strict true / false",
            "state": "HelperRunning",
            "effect": "set",
            "evidence": "tasks 2.3; args contain / omit '--strict'"
          }
        ],
        "forbidden": [
          "helper with inherited stdio",
          "helper call without timeout",
          "discarded teardown result",
          "any test TunRuntime whose helper_path can resolve to the real v2ray-rs-netctl once the helper is actually invoked (use a tempdir stub script)"
        ],
        "seeding": [
          "stub helper = tempdir shell script (0o755) recording `echo \"$1\" >> calls` then the scripted behavior",
          "teardown test: in-module manager, non-TUN start of `exec sleep 30`, then in-module `mgr.tun = Some(TunRuntime{backend: Xray, iface: 'lo', helper_path: stub, strict: true, ..})` - the state a TUN start that passed preflight produces; capability probe cannot pass in tests"
        ],
        "budgets": [
          "HELPER_TIMEOUT 10s production",
          "tests pass 200ms timeout to run_helper; each test wall-clock under 3s"
        ],
        "names": [
          "TunRuntime::strict",
          "HELPER_TIMEOUT",
          "xray_up_args",
          "HelperRun",
          "run_helper",
          "ProcessManager::log_helper",
          "tun::tests::xray_up_args_include_strict_when_set",
          "tun::tests::xray_up_args_omit_strict_when_off",
          "tun::tests::helper_killed_after_timeout",
          "tun::tests::helper_output_is_captured",
          "manager::tests::teardown_failure_is_logged"
        ],
        "refusals": [
          "timeout/exit failure: process layer tun::run_helper at the deadline or exit; surfaced as ProcessError::TunHelper by the manager launch path"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "tun.rs tests first: xray_up_args_include_strict_when_set, xray_up_args_omit_strict_when_off (exact vec: ['xray-up','--iface','tun0','--addr','172.19.0.1/30'] then optional '--addr6' v6, '--bypass-uid' uid, '--capture-dns', '--strict' in that order); helper_killed_after_timeout (stub script writes $$ to a pid file then `exec sleep 30`; run_helper with 200ms timeout returns Err containing 'timed out', elapsed < 2s, `kill(pid, None)` then fails with ESRCH); helper_output_is_captured (stub `echo up-ok; echo 'netctl: boom' >&2; exit 1` -> output has both lines, result Err containing exit status).",
        "manager.rs test: teardown_failure_is_logged (seeding below, helper stub `exit 1`; after stop() state Stopped and log buffer has a line containing 'xray-down failed').",
        "tun.rs: `pub strict: bool` on TunRuntime (doc: install the fail-closed fallback routes; xray only). `pub(crate) const HELPER_TIMEOUT: Duration = Duration::from_secs(10);` `pub(crate) fn xray_up_args(rt: &TunRuntime) -> Vec<String>`; `pub(crate) struct HelperRun { pub output: Vec<String>, pub result: Result<(), String> }`; `pub(crate) async fn run_helper(helper: &Path, args: &[String], timeout: Duration) -> HelperRun` (stdin null, stdout/stderr piped, kill_on_drop(true), wait_with_output under tokio::time::timeout; timeout -> result Err('<verb> timed out after 10s')); xray_up(rt) and xray_down(rt) return HelperRun using HELPER_TIMEOUT.",
        "manager.rs: `fn log_helper(&self, verb: &str, run: &HelperRun)` pushes each output line as LogLine::stderr into log_buffer and emits ProcessEvent::LogLine (pattern manager.rs:192-199), plus '<verb> failed: <e>' on Err. launch path maps Err to ProcessError::TunHelper(e). teardown_tun logs instead of discarding.",
        "connection.rs build_tun_runtime: `strict: settings.tun.strict_route`. Update TunRuntime literals: connection.rs:354, manager.rs:713/799/834, tun.rs:321 (strict: false).",
        "AMENDMENT (fixes review blocker; signature unchanged: run_helper(helper: &Path, args: &[String], timeout: Duration) -> HelperRun, ProcessManager::log_helper still does the LogLine push): run_helper spawns with stdin null, stdout/stderr piped and kill_on_drop(true); two spawned reader tasks own the line buffers (outside the timed future), and only child.wait() runs inside tokio::time::timeout; on elapse it calls child.kill().await (SIGKILL + reap, no zombie), then joins the readers bounded by 500ms, and HelperRun carries the captured lines in `output` plus a failure formatted with the actual value `{timeout:?}` (not a hard-coded 10s). Production timeout const HELPER_TIMEOUT = Duration::from_secs(10). tun::tests::helper_killed_after_timeout uses a 200ms timeout and a stub that sleeps 30s, and after Err asserts nix kill(pid, None) == Err(ESRCH) immediately.",
        "AMENDMENT: while adding `strict: false` to the TunRuntime literals in manager.rs tests (helper_path PathBuf::from(\"v2ray-rs-netctl\")), switch those helper_path values to a nonexistent absolute path under the test tempdir, so no test can reach the installed helper."
      ],
      "coder": "rust-coder",
      "verify": "make test-process && make test-ui && cargo clippy -p v2ray-rs-process -p v2ray-rs-ui --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "2.2",
          "file": "crates/process/src/tun.rs",
          "symbol": "xray_up",
          "anchor": "pub async fn xray_up(rt: &TunRuntime) -> std::io::Result<bool> {",
          "lines": "203-220",
          "change": "10s timeout, kill_on_drop(true), stdout/stderr piped and pushed into LogBuffer as stderr lines (needs log sink from manager); replaces `.status()` with inherited stdio"
        },
        {
          "task": "2.2",
          "file": "crates/process/src/tun.rs",
          "symbol": "xray_down",
          "anchor": "pub async fn xray_down(rt: &TunRuntime) -> std::io::Result<bool> {",
          "lines": "223-231",
          "change": "same bounding/capture as xray_up"
        },
        {
          "task": "2.2",
          "file": "crates/process/src/tun.rs",
          "symbol": "helper timeout const (NEW)",
          "anchor": "pub const DEVICE_TIMEOUT: Duration = Duration::from_secs(10);",
          "lines": "10",
          "change": "NEW const for the 10s helper timeout beside DEVICE_TIMEOUT"
        },
        {
          "task": "2.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::teardown_tun",
          "anchor": "let _ = tun::xray_down(&rt).await;",
          "lines": "559-565",
          "change": "log Err/false result into LogBuffer + ProcessEvent::LogLine instead of discarding"
        },
        {
          "task": "2.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::spawn_process",
          "anchor": "match tun::xray_up(&rt).await {",
          "lines": "336-359",
          "change": "pass log sink to xray_up; error mapping to ProcessError::TunHelper unchanged"
        },
        {
          "task": "2.2",
          "file": "crates/process/src/tun.rs",
          "symbol": "tests",
          "anchor": "fn xray_needs_helper_singbox_does_not() {",
          "lines": "233-333",
          "change": "NEW tokio tests with stub helper script (write_script pattern): sleeps past timeout → returns timeout err promptly; writes stderr → line lands in LogBuffer"
        },
        {
          "task": "2.3",
          "file": "crates/process/src/tun.rs",
          "symbol": "TunRuntime",
          "anchor": "pub capture_dns: bool,",
          "lines": "24-36",
          "change": "add `pub strict: bool`"
        },
        {
          "task": "2.3",
          "file": "crates/process/src/tun.rs",
          "symbol": "xray_up arg building",
          "anchor": "cmd.arg(\"--capture-dns\");",
          "lines": "204-218",
          "change": "emit `--strict` when rt.strict; factor arg list into a pure fn (NEW) so it is unit-testable"
        },
        {
          "task": "2.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "build_tun_runtime",
          "anchor": "capture_dns: backend == BackendType::Xray",
          "lines": "342-370",
          "change": "set `strict: settings.tun.strict_route`"
        },
        {
          "task": "2.3",
          "file": "crates/process/src/manager.rs",
          "symbol": "tests TunRuntime literals",
          "anchor": "use crate::tun::TunRuntime;",
          "lines": "713-721,799-807,834-842",
          "change": "add `strict` field to the three test literals"
        },
        {
          "task": "2.3",
          "file": "crates/process/src/tun.rs",
          "symbol": "tests TunRuntime literal",
          "anchor": "let mk = |backend| TunRuntime {",
          "lines": "321-329",
          "change": "add `strict` field; NEW test asserting built args contain/omit `--strict`"
        },
        {
          "task": "2.2",
          "file": "crates/process/tests/lifecycle.rs",
          "symbol": "integration tests",
          "anchor": "async fn signal_death_is_treated_as_crash",
          "new": true,
          "change": "new test fns after this anchor: may add helper-stub tests here if not in-file"
        }
      ],
      "pkgDirs": [
        "crates/process",
        "crates/ui"
      ],
      "pkgs": [
        "v2ray-rs-process",
        "v2ray-rs-ui"
      ],
      "parallel": true,
      "shard": "",
      "targetedTests": [
        "tun::tests::xray_up_args_include_strict_when_set",
        "tun::tests::xray_up_args_omit_strict_when_off",
        "tun::tests::helper_killed_after_timeout",
        "tun::tests::helper_output_is_captured",
        "manager::tests::teardown_failure_is_logged"
      ]
    },
    {
      "id": "process-state-stop",
      "taskIds": [
        "2.1"
      ],
      "seam": "process-state-stop",
      "contract": {
        "states": [
          "Stopped",
          "Starting",
          "Running",
          "Stopping",
          "Error"
        ],
        "transitions": [
          {
            "input": "stop(), child Some, Running",
            "state": "Stopped",
            "effect": "set",
            "evidence": "manager.rs:271-276: Stopping, SIGTERM, 5s, SIGKILL, teardown, Stopped"
          },
          {
            "input": "stop(), child Some, Starting",
            "state": "Stopped",
            "effect": "set",
            "evidence": "process-lifecycle 'Stop during start'; needs (Starting, Stopping)"
          },
          {
            "input": "stop(), child None, Starting",
            "state": "Stopped",
            "effect": "forced",
            "evidence": "design 'stop() works from every state'"
          },
          {
            "input": "stop(), child None, Running",
            "state": "Stopped",
            "effect": "forced",
            "evidence": "design; wait future cancelled between cleanup_after_exit and handle_unexpected_exit"
          },
          {
            "input": "stop(), child None, Stopping",
            "state": "Stopped",
            "effect": "forced",
            "evidence": "idempotent completion of a cut stop"
          },
          {
            "input": "stop(), child None, Error",
            "state": "Stopped",
            "effect": "clear",
            "evidence": "process-lifecycle 'Stop while in Error with no child' + 'routing state released'; teardown then Error->Stopped (state.rs:33)"
          },
          {
            "input": "stop(), Stopped",
            "state": "Stopped",
            "effect": "no-op",
            "evidence": "process-lifecycle 'Already stopped'"
          }
        ],
        "forbidden": [
          "stop() returning with state other than Stopped",
          "Starting->Stopped direct",
          "any caller other than stop() running teardown_tun"
        ],
        "seeding": [
          "Starting without child: with_backend(SingBox), stub `[ \"$1\" = check ] && exec sleep 30; exec sleep 30`; `tokio::time::timeout(300ms, mgr.start_with_connection(None))` drops during check_config (config-check child killed by kill_on_drop)",
          "Running without child: start `exec sleep 30`; in-module `let mut c = mgr.child.take().unwrap(); c.kill().await.unwrap();`",
          "Error without child: missing binary start (existing manager.rs:737-765); tun stub attached in-module for the release assertion"
        ],
        "budgets": [
          "STOP_TIMEOUT 5s SIGTERM->SIGKILL",
          "each test under 5s wall-clock"
        ],
        "names": [
          "ProcessState::can_transition_to (Starting, Stopping)",
          "state::tests::starting_can_move_to_stopping",
          "manager::tests::stop_from_starting_without_child_reaches_stopped",
          "manager::tests::stop_from_running_without_child_reaches_stopped",
          "manager::tests::stop_from_error_releases_tun_state"
        ],
        "refusals": [
          "none new: invalid transitions still refused by ProcessState::transition with TransitionError::Invalid"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "Tests first. state.rs: starting_can_move_to_stopping. manager.rs: stop_from_starting_without_child_reaches_stopped, stop_from_running_without_child_reaches_stopped, stop_from_error_releases_tun_state (tun stub attached, helper calls file has exactly one 'xray-down'); each asserts subscribe() yields StateChanged to Stopping then Stopped (Error row: Stopped only) and stop() returns Ok.",
        "state.rs can_transition_to: add (Starting, Stopping).",
        "manager.rs stop(): child None and Stopped -> return Ok, no event. child None and Error -> teardown_tun, Error->Stopped. Otherwise: transition to Stopping unless already Stopping; graceful_stop (no-op without child); teardown_tun; Stopped; pid_file.remove. Remove the old early return at manager.rs:262-270.",
        "Keep shutdown() = auto_restart false + stop(). Update existing test stop_recovers_from_error_without_child only if its assertions change (they should not)."
      ],
      "coder": "rust-coder",
      "verify": "make test-process && cargo clippy -p v2ray-rs-process --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "2.1",
          "file": "crates/process/src/state.rs",
          "symbol": "ProcessState::can_transition_to",
          "anchor": "| (Starting, Error(_))",
          "lines": "17-35",
          "change": "add `(Starting, Stopping)`"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/state.rs",
          "symbol": "tests",
          "anchor": "fn valid_transitions_succeed() {",
          "lines": "139-187",
          "change": "add Starting→Stopping case to transition tests"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::stop",
          "anchor": "pub async fn stop(&mut self) -> Result<(), ProcessError> {",
          "lines": "261-277",
          "change": "no child: drive Starting/Running/Error through Stopping to Stopped (Error→Stopped direct today); child path unchanged (SIGTERM→wait→SIGKILL, teardown_tun, Stopped)"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::shutdown",
          "anchor": "pub async fn shutdown(&mut self) {",
          "lines": "285-288",
          "change": "no change; relies on new stop() semantics"
        },
        {
          "task": "2.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "tests",
          "anchor": "async fn stop_recovers_from_error_without_child() {",
          "lines": "736-765",
          "change": "NEW unit tests: stop from Running-without-child and from Starting (set state via mgr.state.transition in-module) → Stopped"
        },
        {
          "task": "2.1",
          "file": "crates/process/tests/lifecycle.rs",
          "symbol": "integration tests",
          "anchor": "async fn signal_death_is_treated_as_crash",
          "new": true,
          "change": "new test fns after this anchor: may add stop-from-state tests"
        }
      ],
      "pkgDirs": [
        "crates/process"
      ],
      "pkgs": [
        "v2ray-rs-process"
      ],
      "parallel": false,
      "shard": "",
      "prev": "helper-bounded",
      "sharedPkg": "crates/process",
      "targetedTests": [
        "state::tests::starting_can_move_to_stopping",
        "manager::tests::stop_from_starting_without_child_reaches_stopped",
        "manager::tests::stop_from_running_without_child_reaches_stopped",
        "manager::tests::stop_from_error_releases_tun_state"
      ]
    },
    {
      "id": "respawn-loop",
      "taskIds": [
        "2.4",
        "2.5"
      ],
      "seam": "respawn-loop",
      "contract": {
        "states": [
          "Running",
          "RespawnWait",
          "Respawning",
          "Error",
          "Stopped"
        ],
        "transitions": [
          {
            "input": "exit while Running, auto_restart false",
            "state": "Error",
            "effect": "set",
            "evidence": "manager.rs:532-535; no helper call"
          },
          {
            "input": "exit while Running, crashes in 60s window < 3",
            "state": "RespawnWait",
            "effect": "set",
            "evidence": "process-lifecycle 'Single crash'; Running->Starting before sleep; no xray-down ('xray TUN routing state survives a respawn')"
          },
          {
            "input": "respawn succeeds",
            "state": "Running",
            "effect": "set",
            "evidence": "Starting->Running; one xray-up when tun needs helper"
          },
          {
            "input": "respawn fails (spawn io, TunDeviceTimeout, TunHelper)",
            "state": "RespawnWait",
            "effect": "no-op",
            "evidence": "process-lifecycle 'Failed respawn is retried'; state stays Starting, crash recorded"
          },
          {
            "input": "crashes in window reach 3",
            "state": "Error",
            "effect": "set",
            "evidence": "'Repeated crashes'; tasks 2.5 no helper call on give-up"
          },
          {
            "input": "Stop while RespawnWait or Respawning (future dropped)",
            "state": "Stopped",
            "effect": "forced",
            "evidence": "'Stop during a crash respawn'; via stop() rows of process-state-stop"
          }
        ],
        "forbidden": [
          "xray-down between a crash and the respawn",
          "version, capability or config check during respawn",
          "Starting->Starting transition",
          "teardown_tun on give-up or on a launch failure"
        ],
        "seeding": [
          "stub backend counts runs: `echo run >> runs; n=$(wc -l < runs); [ $n -le K ] && exit 1; exec sleep 30`",
          "tun attached in-module after a non-TUN start: `mgr.tun = Some(TunRuntime{backend: Xray, iface: 'lo', helper_path: stub, ..})`; 'lo' makes wait_for_device return at once",
          "restart_delay set in-module to 50ms"
        ],
        "budgets": [
          "CRASH_RESTART_DELAY 2s x crash count",
          "MAX_CRASHES 3",
          "CRASH_WINDOW 60s",
          "DEVICE_TIMEOUT 10s untouched",
          "tests: restart_delay 50ms, each under 5s wall-clock"
        ],
        "names": [
          "ProcessManager::launch",
          "ProcessManager::respawn",
          "ProcessManager.restart_delay",
          "manager::tests::respawn_skips_preflight",
          "manager::tests::failed_respawn_is_retried",
          "manager::tests::respawn_budget_exhaustion_errors",
          "manager::tests::stop_during_respawn_wait_reaches_stopped"
        ],
        "refusals": [
          "give-up: process layer handle_unexpected_exit when the window holds MAX_CRASHES, Error message contains '3 crashes within 60s'"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "Tests first (manager.rs, restart_delay 50ms): respawn_skips_preflight (SingBox check stub logs 'check' to a file; backend run 1 exits 1, run 2 sleeps; after wait_and_handle_exit: Running, checks=1, runs=2); failed_respawn_is_retried (tun stub on 'lo', helper fails first xray-up and succeeds second; ends Running; calls = xray-up,xray-up; no xray-down); respawn_budget_exhaustion_errors (backend always exit 1 after first spawn; loop `while mgr.state() == Running { mgr.wait_and_handle_exit().await }`; Error containing '3 crashes'; calls has zero 'xray-down'); stop_during_respawn_wait_reaches_stopped (restart_delay 5s; timeout(300ms, wait_and_handle_exit) then shutdown(); Stopped, calls exactly one 'xray-down', zero 'xray-up').",
        "manager.rs: field `restart_delay: Duration` initialized to CRASH_RESTART_DELAY in new().",
        "Rename spawn_process -> `async fn launch(&mut self) -> Result<(), ProcessError>`: on device timeout or xray-up failure run graceful_stop only, no teardown_tun (drop manager.rs:343,350,355 teardown calls).",
        "Add `async fn respawn(&mut self) -> Result<(), ProcessError>`: `self.launch().await?` then Starting->Running with current_connection. Reuses binary_path, config_path, geodata_dir, tun, current_connection unchanged since the start; skips xray_version_triple, has_net_admin, check_config.",
        "handle_unexpected_exit: record crash; if !auto_restart -> Error(msg), return. Loop: if crash_times.len() >= MAX_CRASHES -> Error('{MAX_CRASHES} crashes within {CRASH_WINDOW:?}: {msg}'), return; if state is Running -> transition Starting(current_connection) (never Starting->Starting); sleep(restart_delay * crashes); respawn(): Ok -> return; Err(e) -> log 'restart failed: {e}', msg = that, record crash, continue. Remove the pre-respawn teardown_tun and its comment (manager.rs:527-530).",
        "AMENDMENT: respawn_skips_preflight also asserts a fresh ProcessManager::new(..) has restart delay == CRASH_RESTART_DELAY (the 2s spec default), since the other respawn tests override it to 50ms."
      ],
      "coder": "rust-coder",
      "verify": "make test-process && cargo clippy -p v2ray-rs-process --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "2.4",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::handle_unexpected_exit",
          "anchor": "async fn handle_unexpected_exit(&mut self, exit_code: Option<i32>) {",
          "lines": "509-557",
          "change": "loop: record crash, give up at MAX_CRASHES, sleep CRASH_RESTART_DELAY×n, respawn; respawn Err records another crash and continues"
        },
        {
          "task": "2.4",
          "file": "crates/process/src/manager.rs",
          "symbol": "handle_unexpected_exit pre-respawn teardown",
          "anchor": "// Roll back any TUN routing state the dead backend left behind before we",
          "lines": "527-530",
          "change": "remove comment + `self.teardown_tun().await`"
        },
        {
          "task": "2.4",
          "file": "crates/process/src/manager.rs",
          "symbol": "respawn (NEW)",
          "anchor": "NEW",
          "change": "NEW respawn path: Starting transition + spawn_process, skipping xray_version_triple, has_net_admin probe and check_config (preflight blocks at start_with_connection 157-245)"
        },
        {
          "task": "2.4",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::start_with_connection",
          "anchor": "pub async fn start_with_connection(",
          "lines": "153-259",
          "change": "source of preflight blocks to split from spawn; public behavior unchanged"
        },
        {
          "task": "2.4",
          "file": "crates/process/src/manager.rs",
          "symbol": "spawn_process failure teardown",
          "anchor": "return Err(ProcessError::TunDeviceTimeout(rt.iface.clone()));",
          "lines": "341-357",
          "change": "calls graceful_stop+teardown_tun on device timeout / xray-up failure; on the respawn path this would run xray-down between crash and respawn — must be skipped there"
        },
        {
          "task": "2.4",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::wait_and_handle_exit",
          "anchor": "if self.state.state() == ProcessState::Running {",
          "lines": "294-320",
          "change": "entry to crash path; unchanged"
        },
        {
          "task": "2.4",
          "file": "crates/process/src/manager.rs",
          "symbol": "tests",
          "anchor": "async fn crash_error_includes_last_stderr_line() {",
          "lines": "681-697",
          "change": "NEW tests via manager_for stub backend + stub helper logging invocations to a file: respawn failure retried, budget exhaustion → Error, no xray-down between crash and respawn"
        },
        {
          "task": "2.5",
          "file": "crates/process/src/manager.rs",
          "symbol": "handle_unexpected_exit give-up",
          "anchor": "\"{MAX_CRASHES} crashes within {CRASH_WINDOW:?}: {msg}\"",
          "lines": "532-545",
          "change": "give-up (budget and !auto_restart) leaves routes; no teardown; teardown only in stop()"
        },
        {
          "task": "2.5",
          "file": "crates/process/src/manager.rs",
          "symbol": "tests",
          "anchor": "NEW",
          "change": "NEW unit test: give-up with TunRuntime(xray, stub helper) → helper never invoked"
        },
        {
          "task": "2.4",
          "file": "crates/process/tests/lifecycle.rs",
          "symbol": "integration tests",
          "anchor": "async fn signal_death_is_treated_as_crash",
          "new": true,
          "change": "new test fns after this anchor: crash-loop tests with stub backend scripts"
        }
      ],
      "pkgDirs": [
        "crates/process"
      ],
      "pkgs": [
        "v2ray-rs-process"
      ],
      "parallel": false,
      "shard": "",
      "prev": "process-state-stop",
      "sharedPkg": "crates/process",
      "targetedTests": [
        "manager::tests::respawn_skips_preflight",
        "manager::tests::failed_respawn_is_retried",
        "manager::tests::respawn_budget_exhaustion_errors",
        "manager::tests::stop_during_respawn_wait_reaches_stopped"
      ]
    },
    {
      "id": "conn-terminal-state",
      "taskIds": [
        "3.1",
        "3.2"
      ],
      "seam": "conn-terminal-state",
      "contract": {
        "states": [
          "Starting",
          "Running",
          "Stopping",
          "Stopped",
          "Error"
        ],
        "transitions": [
          {
            "input": "manager StateChanged to Starting|Running|Stopping",
            "state": "Running",
            "effect": "set",
            "evidence": "design 'Terminal states come only from the supervising task'; relayed with the generation"
          },
          {
            "input": "manager StateChanged to Stopped|Error",
            "state": "Running",
            "effect": "no-op",
            "evidence": "connection.rs:210 extended to Stopped"
          },
          {
            "input": "candidate gives up, more candidates remain",
            "state": "Starting",
            "effect": "no-op",
            "evidence": "'Failover is not reported as a stop': no Stopped, handle and marker kept"
          },
          {
            "input": "Stop at any point (queued, pre-start, during start, supervising, after failover)",
            "state": "Stopped",
            "effect": "forced",
            "evidence": "tasks 3.1; exactly one Stopped"
          },
          {
            "input": "last candidate fails",
            "state": "Error",
            "effect": "set",
            "evidence": "'Last candidate fails': one Error(summarize_failures)"
          },
          {
            "input": "TunRuntime Some before start",
            "state": "Starting",
            "effect": "set",
            "evidence": "tun-mode 'Marker precedes route changes'; TunSession{backend: rt.backend, iface: rt.iface}"
          }
        ],
        "forbidden": [
          "two terminal messages for one generation",
          "any message for the generation after its terminal one",
          "Stopped on failover",
          "marker written after start_with_connection or from settings"
        ],
        "seeding": [
          "AppPaths::for_profile_in(AppProfile::Test, tmp); ConfigWriter::new(&settings, &paths); settings.backend.binary_path unused (request.binary_path = stub)",
          "candidates: Shadowsocks 203.0.113.1 and 203.0.113.2 (no DNS pin lookup)",
          "stub backend: `[ \"$1\" = check ] && exit 0; grep -q 203.0.113.1 \"$3\" && exit 1; exec sleep 30`, backend SingBox, TUN off",
          "marker test: tun.enabled, backend Xray, stub without caps -> start fails at the capability gate after the marker write"
        ],
        "budgets": [
          "crash-looping candidate costs 2s+4s real (restart_delay not exposed across crates, by choice); per-message recv timeout 20s; test wall-clock under 30s"
        ],
        "names": [
          "relays",
          "tun_session_for",
          "ConnectionRequest.paths",
          "connection::tests::forwarder_relays_only_nonterminal_states",
          "connection::tests::failover_reports_no_stopped_and_stop_reports_one",
          "connection::tests::last_candidate_failure_reports_one_error",
          "connection::tests::marker_written_from_runtime_before_start"
        ],
        "refusals": [
          "Stop: connection task select at the first await after the command; manager stop() does the release"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "Tests first (connection.rs, #[tokio::test(flavor = 'multi_thread')]): forwarder_relays_only_nonterminal_states (pure); failover_reports_no_stopped_and_stop_reports_one; last_candidate_failure_reports_one_error; marker_written_from_runtime_before_start. No manager trait: the seam is connection::spawn with a stub backend script and `relm4::channel::<AppMsg>()` (re-exported by relm4 0.10; confirm) as the sender.",
        "`fn relays(state: &ProcessState) -> bool` = Starting|Running|Stopping; forwarder uses it; keep its JoinHandle.",
        "Before every terminal emit and before failing over: `forwarder.abort(); let _ = forwarder.await;` so no relayed state follows the terminal one.",
        "Failover and start-failure paths drop shutdown(): keep the failed manager as `parked: Option<ProcessManager>` (replaced per candidate, dropped at final Error). Stop at loop top or pre-start: `if let Some(mut m) = parked.take() { m.shutdown().await }` then emit Stopped, so leftover routes are released.",
        "Wrap start_with_connection in `tokio::select! { biased; Some(ConnectionCmd::Stop) = cmd_rx.recv() => { mgr.shutdown().await; emit Stopped; return } r = mgr.start_with_connection(..) => r }`. Supervise loop Stop branch: shutdown, abort forwarder, emit Stopped, return.",
        "ConnectionRequest gains `pub paths: AppPaths`; app.rs start_connection passes self.paths.clone(). `fn tun_session_for(rt: &TunRuntime) -> TunSession`. Per candidate, when the runtime is Some, save_tun_session before start_with_connection; failure -> log::warn. Delete the marker write at app.rs:1128-1142.",
        "AMENDMENT: marker_written_from_runtime_before_start uses a single candidate and a stub that answers `version` at once (`[ \"$1\" = version ] && echo \"Xray 26.6.27\" && exit 0`), so it never waits CONFIG_CHECK_TIMEOUT.",
        "AMENDMENT: parking a failed manager aborts its log-forwarder task together with its state forwarder, so a failed candidate cannot emit log lines after the next candidate starts."
      ],
      "coder": "rust-coder",
      "verify": "make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "3.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "per-manager state forwarder",
          "anchor": "if !matches!(to, ProcessState::Error(_)) {",
          "lines": "201-221",
          "change": "relay only Starting/Running/Stopping"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "select! Stop arm",
          "anchor": "ConnectionCmd::Stop => {",
          "lines": "238-248",
          "change": "after mgr.shutdown() emit ProcessStateConnection(generation, Stopped, None)"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "select! wait_and_handle_exit arm",
          "anchor": "stop_requested = true;",
          "lines": "249-270",
          "change": "emit Stopped on the stop_requested return path"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "failover loop",
          "anchor": "failures.push(format!(\"{candidate_label}: {e}\"));",
          "lines": "106-272",
          "change": "start failure/give-up shutdown must not surface Stopped (forwarder filter); early-return Stop paths 84-91,107-114,179-186 already emit Stopped"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "final Error emit",
          "anchor": "let msg = summarize_failures(&failures);",
          "lines": "274-279",
          "change": "single Error after last candidate; unchanged"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests",
          "anchor": "fn pinned_hostname_arms_capture() {",
          "lines": "391-467",
          "change": "NEW failover-sequence test; needs a manager seam (connection::spawn constructs ProcessManager inline at 165) — see MISSING stub manager"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "marker write before start",
          "anchor": "match mgr.start_with_connection(Some(meta.clone())).await {",
          "lines": "165-174",
          "change": "bind build_tun_runtime result, save TunSession{backend,iface} from it before start_with_connection"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "ConnectionRequest",
          "anchor": "pub generation: u64,",
          "lines": "32-48",
          "change": "add a field carrying AppPaths (or marker path) for the marker write"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::start_connection",
          "anchor": "lifecycle: self.tun_lifecycle.clone(),",
          "lines": "477-492",
          "change": "pass paths into ConnectionRequest"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "ProcessStateConnection marker write on Running",
          "anchor": "let session = v2ray_rs_core::persistence::TunSession {",
          "lines": "1128-1142",
          "change": "remove marker write"
        },
        {
          "task": "3.2",
          "file": "crates/core/src/persistence/tun_session.rs",
          "symbol": "save_tun_session",
          "anchor": "pub fn save_tun_session(paths: &AppPaths, session: &TunSession) -> Result<(), PersistenceError> {",
          "lines": "17-21",
          "change": "reused as-is; path = state_dir/tun_session.json (persistence/mod.rs:234)"
        }
      ],
      "pkgDirs": [
        "crates/ui"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "parallel": false,
      "shard": "",
      "prev": "respawn-loop",
      "sharedPkg": "crates/ui",
      "targetedTests": [
        "connection::tests::forwarder_relays_only_nonterminal_states",
        "connection::tests::failover_reports_no_stopped_and_stop_reports_one",
        "connection::tests::last_candidate_failure_reports_one_error",
        "connection::tests::marker_written_from_runtime_before_start"
      ]
    },
    {
      "id": "app-killswitch",
      "taskIds": [
        "3.3",
        "3.4"
      ],
      "seam": "app-killswitch",
      "contract": {
        "states": [
          "Idle",
          "Live",
          "StopInFlight",
          "Held",
          "Releasing",
          "Exiting"
        ],
        "transitions": [
          {
            "input": "Error, retry follows",
            "state": "Held",
            "effect": "no-op",
            "evidence": "tun-mode 'Blocked across automatic reconnects'"
          },
          {
            "input": "Error, attempts >= 3 or no retry path",
            "state": "Releasing",
            "effect": "set",
            "evidence": "'Released on final give-up'; state stays Error"
          },
          {
            "input": "Error while app state was Stopping",
            "state": "Releasing",
            "effect": "set",
            "evidence": "user already disconnected; no auto-reconnect"
          },
          {
            "input": "Disconnect, no handle, marker present",
            "state": "Releasing",
            "effect": "set",
            "evidence": "'Released on Disconnect without a live backend'; reports Stopped"
          },
          {
            "input": "Disconnect, no handle, no marker",
            "state": "Idle",
            "effect": "no-op",
            "evidence": "app.rs:1090-1093"
          },
          {
            "input": "Quit, handle Some",
            "state": "Exiting",
            "effect": "set",
            "evidence": "app.rs:1211-1213; destroy on Stopped"
          },
          {
            "input": "Quit, no handle, app state Stopping",
            "state": "Exiting",
            "effect": "forced",
            "evidence": "'Quit during a stop'; tasks 3.4; today app.rs:1214 destroys at once"
          },
          {
            "input": "Quit, no handle, marker present",
            "state": "Releasing",
            "effect": "set",
            "evidence": "tun-mode 'released on Quit'; destroy on TunReleased"
          },
          {
            "input": "Quit idle",
            "state": "Idle",
            "effect": "clear",
            "evidence": "window.destroy"
          },
          {
            "input": "TunReleased",
            "state": "Idle",
            "effect": "clear",
            "evidence": "marker cleared by recover_tun_session; destroy if pending_exit"
          }
        ],
        "forbidden": [
          "window destroyed while app state Stopping or a release in flight",
          "marker cleared while a retry is scheduled",
          "Connect starting while tun_release_in_flight",
          "release while a handle is Some"
        ],
        "seeding": [
          "pure fns take plain args",
          "recover tests: AppPaths::for_profile_in(AppProfile::Test, tmp) + save_tun_session(TunSession{backend: Xray, iface: 'tun9'}) + tempdir stub helper"
        ],
        "budgets": [
          "RECOVER_TIMEOUT 5s",
          "MAX_AUTO_RECONNECTS 3",
          "AUTO_RECONNECT_DELAY 5s",
          "each test under 2s"
        ],
        "names": [
          "QuitPlan",
          "quit_plan",
          "auto_reconnect_allowed",
          "release_tun_session",
          "tun_release_in_flight",
          "AppMsg::TunReleased",
          "recover_tun_session(paths, helper)",
          "app::tests::quit_while_stopping_waits_for_stopped",
          "app::tests::quit_with_handle_stops_first",
          "app::tests::quit_with_leftover_marker_releases_first",
          "app::tests::quit_idle_exits",
          "app::tests::auto_reconnect_exhausted_after_three_attempts",
          "app::tests::auto_reconnect_suppressed_on_exit",
          "app::tests::recover_runs_helper_and_clears_marker",
          "app::tests::recover_clears_marker_when_helper_fails",
          "disconnect_plan",
          "DisconnectPlan { Stop, Release, Nothing }",
          "release_on_error"
        ],
        "refusals": [
          "helper hang: run_with_timeout kills at RECOVER_TIMEOUT, marker cleared regardless (app.rs:1590-1598)"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "Tests first (app.rs pure fns): quit_while_stopping_waits_for_stopped, quit_with_handle_stops_first, quit_with_leftover_marker_releases_first, quit_idle_exits, auto_reconnect_exhausted_after_three_attempts, auto_reconnect_suppressed_on_exit, recover_runs_helper_and_clears_marker (stub helper writes \"$@\"; args exactly 'recover --xray --iface tun9'), recover_clears_marker_when_helper_fails (stub exit 1).",
        "`recover_tun_session(paths: &AppPaths, helper: &Path)`; init caller passes &v2ray_rs_process::helper_path(). Tests never run the real helper.",
        "`enum QuitPlan { Stop, AwaitStopped, Release, Exit }`, `fn quit_plan(has_handle: bool, state: &ProcessState, marker_present: bool) -> QuitPlan`; TrayQuit and CloseRequested (non-tray branch) use it; AwaitStopped and Release set pending_exit.",
        "`fn auto_reconnect_allowed(pending_exit: bool, attempts: u32) -> bool`; schedule_auto_reconnect returns bool.",
        "`fn release_tun_session(&mut self, sender)`: sets `tun_release_in_flight`, tokio::spawn { lifecycle.lock(); spawn_blocking(recover_tun_session) } then `AppMsg::TunReleased`. Handler clears the flag; destroys window if pending_exit. Connect and ConnectToNode return early while the flag is set.",
        "ProcessStateConnection Error: release when no retry follows (no reconnect_pending, no replay target, auto_reconnect_allowed false or not scheduled, direct_connect_in_flight, pending_exit, or app state was Stopping). Disconnect without handle and marker present: cancel_auto_reconnect, apply_state(Stopped), release; else existing 'Not connected' toast.",
        "AMENDMENT: extract pure fns beside quit_plan: disconnect_plan(has_handle: bool, marker_present: bool) -> DisconnectPlan { Stop, Release, Nothing } and release_on_error(app_state_stopping: bool, reconnects_left: u32) -> bool — the retry-budget sub-decision only; the Error handler still combines it with reconnect_pending, the replay target, direct_connect_in_flight and pending_exit as today. The Stopped report after a Release rests on apply_state(Stopped) in the handler (accepted gap, no handler-level test). Tests app::tests::disconnect_without_handle_releases_marker and app::tests::error_while_stopping_releases_without_retry, plus app::tests::error_with_reconnects_left_keeps_killswitch."
      ],
      "coder": "rust-coder",
      "verify": "make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "recover_tun_session",
          "anchor": "fn recover_tun_session(paths: &AppPaths) {",
          "lines": "1577-1599",
          "change": "reused release pass (blocking, run_with_timeout RECOVER_TIMEOUT=5s); must run via spawn_blocking under tun_lifecycle lock like init 750-765"
        },
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "startup route-recovery pass",
          "anchor": "recover_tun_session(&bg_paths);",
          "lines": "750-765",
          "change": "pattern to reuse for release; unchanged"
        },
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "Disconnect with no handle",
          "anchor": "self.show_toast(\"Not connected\");",
          "lines": "1084-1094",
          "change": "run release pass + clear marker in the no-handle branch"
        },
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::schedule_auto_reconnect (reconnect budget)",
          "anchor": "if self.pending_exit || self.auto_reconnect_attempts >= MAX_AUTO_RECONNECTS {",
          "lines": "241-251",
          "change": "on budget exhausted run release pass (MAX_AUTO_RECONNECTS=3, app.rs:35)"
        },
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "ProcessStateConnection terminal handling",
          "anchor": "let _ = v2ray_rs_core::persistence::clear_tun_session(&self.paths);",
          "lines": "1102-1113,1169-1185",
          "change": "marker clear-on-Stopped and Error→schedule_auto_reconnect / direct_connect_in_flight branches: route release decisions through the new pass"
        },
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "TrayQuit / CloseRequested",
          "anchor": "AppMsg::TrayQuit => {",
          "lines": "1196-1217",
          "change": "no-handle quit runs release pass before window.destroy()"
        },
        {
          "task": "3.4",
          "file": "crates/ui/src/app.rs",
          "symbol": "TrayQuit / CloseRequested pending_exit",
          "anchor": "AppMsg::CloseRequested => {",
          "lines": "1196-1217",
          "change": "when handle is None but process_state == Stopping (Disconnect took it at 1087), set pending_exit instead of destroying"
        },
        {
          "task": "3.4",
          "file": "crates/ui/src/app.rs",
          "symbol": "pending_exit consumer",
          "anchor": "if stopped && self.pending_exit {",
          "lines": "1147-1151",
          "change": "window destroyed here only after Stopped; unchanged"
        }
      ],
      "pkgDirs": [
        "crates/ui"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "parallel": false,
      "shard": "",
      "prev": "conn-terminal-state",
      "sharedPkg": "crates/ui",
      "targetedTests": [
        "app::tests::quit_while_stopping_waits_for_stopped",
        "app::tests::quit_with_handle_stops_first",
        "app::tests::quit_with_leftover_marker_releases_first",
        "app::tests::quit_idle_exits",
        "app::tests::auto_reconnect_exhausted_after_three_attempts",
        "app::tests::auto_reconnect_suppressed_on_exit",
        "app::tests::recover_runs_helper_and_clears_marker",
        "app::tests::recover_clears_marker_when_helper_fails",
        "app::tests::disconnect_without_handle_releases_marker",
        "app::tests::error_while_stopping_releases_without_retry",
        "app::tests::error_with_reconnects_left_keeps_killswitch"
      ]
    },
    {
      "id": "cancel-starting",
      "taskIds": [
        "3.5"
      ],
      "seam": "cancel-starting",
      "contract": {
        "states": [
          "Stopped",
          "Starting",
          "Running",
          "Stopping",
          "Error"
        ],
        "transitions": [
          {
            "input": "state Starting (incl. respawn)",
            "state": "Starting",
            "effect": "set",
            "evidence": "ui-statusbar-logs 'Starting state button is actionable'; system-tray 'Disconnect enabled while starting'"
          },
          {
            "input": "state Stopping",
            "state": "Stopping",
            "effect": "set",
            "evidence": "system-tray 'disabled during transitions'"
          },
          {
            "input": "Disconnect during Starting",
            "state": "Stopped",
            "effect": "forced",
            "evidence": "'Cancel a slow connect'; conn-terminal-state select"
          }
        ],
        "forbidden": [
          "Connect offered while Starting",
          "enabled item while Stopping"
        ],
        "seeding": [
          "pure fns take a ProcessState"
        ],
        "budgets": [
          "tests under 1s"
        ],
        "names": [
          "connect_toggle",
          "toggle_action"
        ],
        "refusals": [
          "none"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "Tests first: app::tests::toggle_is_actionable_disconnect_while_starting, app::tests::toggle_disabled_while_stopping, tray::tests::toggle_is_enabled_disconnect_while_starting, tray::tests::toggle_disabled_while_stopping (TrayAction has no PartialEq: use matches!).",
        "app.rs `fn connect_toggle(state: &ProcessState) -> (bool, bool)` = (shows_disconnect, sensitive): Stopped (false,true), Starting (true,true), Running (true,true), Stopping (true,false), Error (false,true); apply_state sets connected/button_sensitive from it. `connected` is read only by the button and ToggleConnection.",
        "tray.rs `fn toggle_action(state: &ProcessState) -> (TrayAction, bool)`: Running|Starting (Disconnect,true), Stopping (Connect,false), Stopped|Error (Connect,true); menu() builds the item from it."
      ],
      "coder": "rust-coder",
      "verify": "make test-ui && make test-tray && cargo clippy -p v2ray-rs-ui -p v2ray-rs-tray --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "3.5",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::apply_state (connect button state)",
          "anchor": "fn apply_state(&mut self, state: &ProcessState) {",
          "lines": "126-186",
          "change": "Starting → button sensitive with Disconnect label (today connected=false, button_sensitive=false at 133-136)"
        },
        {
          "task": "3.5",
          "file": "crates/ui/src/app.rs",
          "symbol": "connect button view",
          "anchor": "set_label: if model.connected { \"Disconnect\" } else { \"Connect\" },",
          "lines": "684-706",
          "change": "label/icon/css/tooltip keyed on model.connected; adjust so Starting shows Disconnect"
        },
        {
          "task": "3.5",
          "file": "crates/ui/src/app.rs",
          "symbol": "ToggleConnection",
          "anchor": "AppMsg::ToggleConnection => {",
          "lines": "975-981",
          "change": "dispatch Disconnect while Starting"
        },
        {
          "task": "3.5",
          "file": "crates/tray/src/tray.rs",
          "symbol": "AppTray::menu",
          "anchor": "let connected = self.process_state == ProcessState::Running;",
          "lines": "97-123",
          "change": "Starting → enabled Disconnect item; Stopping stays disabled"
        },
        {
          "task": "3.5",
          "file": "crates/tray/src/tray.rs",
          "symbol": "tests",
          "anchor": "fn tooltip_description_includes_manual_source() {",
          "lines": "370-408",
          "change": "NEW menu-state test (label/enabled for Starting); no existing menu test"
        }
      ],
      "pkgDirs": [
        "crates/ui",
        "crates/tray"
      ],
      "pkgs": [
        "v2ray-rs-ui",
        "v2ray-rs-tray"
      ],
      "parallel": false,
      "shard": "",
      "prev": "app-killswitch",
      "sharedPkg": "crates/ui",
      "targetedTests": [
        "app::tests::toggle_is_actionable_disconnect_while_starting",
        "app::tests::toggle_disabled_while_stopping",
        "tray::tests::toggle_is_enabled_disconnect_while_starting",
        "tray::tests::toggle_disabled_while_stopping"
      ]
    },
    {
      "id": "prefs-docs",
      "taskIds": [
        "4.1",
        "4.2"
      ],
      "seam": "prefs-docs",
      "contract": {
        "states": [
          "StrictSensitive",
          "StrictInsensitive"
        ],
        "transitions": [
          {
            "input": "backend Xray or SingBox",
            "state": "StrictSensitive",
            "effect": "set",
            "evidence": "proposal 'strict_route applies to xray'"
          },
          {
            "input": "backend V2ray",
            "state": "StrictInsensitive",
            "effect": "set",
            "evidence": "no TUN for v2ray"
          }
        ],
        "forbidden": [
          "note claiming strict is sing-box only"
        ],
        "seeding": [
          "pure fn"
        ],
        "budgets": [
          "test under 1s"
        ],
        "names": [
          "strict_route_applies",
          "preferences::tun::tests::strict_route_row_sensitive_for_xray"
        ],
        "refusals": [
          "none"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "preferences/tun.rs new test module: strict_route_row_sensitive_for_xray (strict_route_applies(Xray) and (SingBox) true, (V2ray) false).",
        "`fn strict_route_applies(backend: BackendType) -> bool`; strict_row.set_sensitive uses it at tun.rs:725 and :736; strict row subtitle states it applies to both backends and, under xray, blocks traffic while reconnecting and IPv6 without an IPv6 tunnel address; advanced_note (tun.rs:126-130) narrowed so it no longer claims strict is sing-box only.",
        "docs/ARCHITECTURE.md: state machine (104-106: Starting->Stopping), crash recovery (108-118: respawn loop, preflight reuse, no teardown), netctl (148-165: --strict, v6 rules, xray-down clears table 2023), stop/marker (229-238: teardown only from stop, release owned by the app, marker written by the connection task before start).",
        "CHANGELOG.md [Unreleased] Changed: BREAKING for xray TUN with strict_route on (default): IPv6 refused without an IPv6 tunnel address; host blocked while reconnecting. Fixed: Disconnect during respawn/start, failover handle loss, helper timeout."
      ],
      "coder": "rust-coder",
      "verify": "make test-ui && rg -n strict_route docs/ARCHITECTURE.md CHANGELOG.md",
      "sites": [
        {
          "task": "4.1",
          "file": "crates/ui/src/preferences/tun.rs",
          "symbol": "strict_row",
          "anchor": ".title(\"Strict route\")",
          "lines": "140-144",
          "change": "add subtitle: applies to both backends; under xray blocks traffic while the tunnel is down and IPv6 without an IPv6 tunnel address"
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/preferences/tun.rs",
          "symbol": "backend gating",
          "anchor": "let strict_row = strict_row.clone();",
          "lines": "709-736",
          "change": "strict_row.set_sensitive(singbox_only) at 725 and 736 → sensitive for xray too"
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/preferences/tun.rs",
          "symbol": "advanced_note",
          "anchor": ".title(\"These options apply to sing-box only\")",
          "lines": "126-131",
          "change": "note becomes inaccurate once strict route applies to xray; reword or exempt"
        },
        {
          "task": "4.2",
          "file": "docs/ARCHITECTURE.md",
          "symbol": "process crash recovery",
          "anchor": "Crash recovery: any exit while the state is still Running counts as an",
          "lines": "104-118",
          "change": "state machine gains Starting→Stopping; crash path: no teardown, respawn loop with preflight reuse, budget covers respawn failures"
        },
        {
          "task": "4.2",
          "file": "docs/ARCHITECTURE.md",
          "symbol": "netctl xray-up",
          "anchor": "- `xray-up --iface --addr [--addr6] [--bypass-uid] [--capture-dns]`: brings the",
          "lines": "146-165",
          "change": "document `--strict`: unreachable fallback per family, IPv6 rules without --addr6; xray-down/recover remove them"
        },
        {
          "task": "4.2",
          "file": "docs/ARCHITECTURE.md",
          "symbol": "xray stop/teardown + marker",
          "anchor": "Stop: SIGTERM lets xray close its TUN fd (kernel drops device-scoped routes),",
          "lines": "229-238",
          "change": "teardown only from stop(); kill-switch release owned by app (give-up/Disconnect/Quit); marker written by connection task before routes"
        },
        {
          "task": "4.2",
          "file": "CHANGELOG.md",
          "symbol": "[Unreleased]",
          "anchor": "## [Unreleased]",
          "lines": "8-18",
          "change": "add entries (Changed/Fixed) incl. BREAKING IPv6 under strict_route for xray"
        }
      ],
      "pkgDirs": [
        "crates/ui"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "parallel": false,
      "shard": "",
      "prev": "cancel-starting",
      "sharedPkg": "crates/ui",
      "targetedTests": [
        "preferences::tun::tests::strict_route_row_sensitive_for_xray"
      ]
    },
    {
      "id": "verify-floor",
      "taskIds": [
        "5.1",
        "5.2"
      ],
      "seam": "verification",
      "contract": {
        "states": [
          "FloorGreen",
          "HandedOff"
        ],
        "transitions": [
          {
            "input": "floor passes",
            "state": "FloorGreen",
            "effect": "set",
            "evidence": "tasks 5.1"
          },
          {
            "input": "privileged or live check pending",
            "state": "HandedOff",
            "effect": "no-op",
            "evidence": "tasks 5.2-5.4 need root / a live host"
          }
        ],
        "forbidden": [
          "marking 5.2-5.4 done without a user report"
        ],
        "seeding": [
          "none"
        ],
        "budgets": [
          "floor 10m, 4 threads",
          "privileged 5m"
        ],
        "names": [],
        "refusals": [
          "none"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [],
      "coder": "zpatcher",
      "verify": "make lint && make test TEST_TIMEOUT=10m",
      "sites": [
        {
          "task": "5.1",
          "file": "Makefile",
          "symbol": "test target",
          "anchor": "TEST_THREADS ?= 4",
          "lines": "14-17,108-110",
          "change": "runner: `make test` = `timeout 5m cargo test --workspace --all-targets -- --test-threads=4` (task asks 10m: `TEST_TIMEOUT=10m make test`); exercises crates/process/tests/lifecycle.rs + in-file mod tests in state.rs, manager.rs, tun.rs, connection.rs, app.rs, tray.rs"
        },
        {
          "task": "5.2",
          "file": "crates/netctl/tests/privileged.rs",
          "symbol": "module doc run line",
          "anchor": "//! Run with: `sudo -E cargo test -p v2ray-rs-netctl --features privileged-tests`.",
          "lines": "1-6",
          "change": "only documented runner (no script); feature gate `#![cfg(feature = \"privileged-tests\")]`, feature declared crates/netctl/Cargo.toml:20"
        }
      ],
      "pkgDirs": [],
      "pkgs": [],
      "parallel": false,
      "shard": "",
      "prev": "prefs-docs",
      "sharedPkg": "workspace",
      "targetedTests": []
    },
    {
      "id": "verify-live",
      "taskIds": [
        "5.3",
        "5.4"
      ],
      "seam": "verification",
      "contract": {
        "states": [
          "FloorGreen",
          "HandedOff"
        ],
        "transitions": [
          {
            "input": "floor passes",
            "state": "FloorGreen",
            "effect": "set",
            "evidence": "tasks 5.1"
          },
          {
            "input": "privileged or live check pending",
            "state": "HandedOff",
            "effect": "no-op",
            "evidence": "tasks 5.2-5.4 need root / a live host"
          }
        ],
        "forbidden": [
          "marking 5.2-5.4 done without a user report"
        ],
        "seeding": [
          "none"
        ],
        "budgets": [
          "floor 10m, 4 threads",
          "privileged 5m"
        ],
        "names": [],
        "refusals": [
          "none"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [],
      "coder": "zpatcher",
      "verify": "manual: live xray TUN host checks 5.3/5.4, run by the user after merge",
      "sites": [
        {
          "task": "5.3",
          "file": "crates/netctl/src/net.rs",
          "symbol": "XRAY_ROUTE_TABLE",
          "anchor": "const XRAY_ROUTE_TABLE: u32 = 2023;",
          "change": "manual live check only; no code; inspect table 2023 / `ip route get` during SEGV gap"
        },
        {
          "task": "5.4",
          "file": "crates/netctl/src/net.rs",
          "symbol": "RULE_PREF_BYPASS_UID",
          "anchor": "const RULE_PREF_BYPASS_UID: u32 = 8998;",
          "change": "manual live check only; `ip rule` has no 8998-9002 and table 2023 empty after Disconnect in respawn window"
        }
      ],
      "pkgDirs": [],
      "pkgs": [],
      "parallel": false,
      "shard": "",
      "prev": "verify-floor",
      "sharedPkg": "workspace",
      "targetedTests": []
    }
  ],
  "seams": [
    {
      "id": "netctl-strict",
      "tasks": [
        "1.1",
        "1.2",
        "1.3"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. xray-up --strict adds max-metric unreachable defaults (v4+v6) to table 2023 and v6 policy rules without --addr6; xray-down/recover flush table 2023 and prefs 8998-9002 for both families.",
      "contract": {
        "states": [
          "Clean",
          "Up",
          "UpStrict",
          "DeviceGone"
        ],
        "transitions": [
          {
            "input": "xray-up without --strict, without --addr6",
            "state": "Up",
            "effect": "set",
            "evidence": "net.rs:61-105; tun-mode 'Strict route off keeps previous behavior': v4 device route + v4 9000/9001/9002 only, no unreachable, no v6 rules"
          },
          {
            "input": "xray-up --strict without --addr6",
            "state": "UpStrict",
            "effect": "set",
            "evidence": "tun-mode 'Strict fallback routes': unreachable default metric 4294967295 table 2023 v4+v6; v6 9000/9001/9002 (+8998 with --bypass-uid); no v6 8999, no v6 device route"
          },
          {
            "input": "xray-up --strict --addr6",
            "state": "UpStrict",
            "effect": "set",
            "evidence": "net.rs:81-102 plus fallback routes; v6 8999 only with --capture-dns"
          },
          {
            "input": "TUN device deleted while UpStrict",
            "state": "DeviceGone",
            "effect": "forced",
            "evidence": "design 'Fail closed with a fallback route': kernel drops device routes, unreachable answers pref 9002; pref 9000 mark 255 and 8998 still reach main"
          },
          {
            "input": "xray-up --strict on recreated device",
            "state": "UpStrict",
            "effect": "no-op",
            "evidence": "EEXIST tolerance net.rs:160-283; tun-mode 'Re-running up across a recreated device'"
          },
          {
            "input": "xray-down from any state",
            "state": "Clean",
            "effect": "clear",
            "evidence": "tun-mode 'Tear xray TUN routes down': prefs 8998-9002 both families, table 2023 flushed both families, TUN device deleted; no-op when absent"
          },
          {
            "input": "recover --xray from any state",
            "state": "Clean",
            "effect": "clear",
            "evidence": "net.rs:132-136; tun-mode 'Recover leftovers'"
          },
          {
            "input": "xray-up on non-TUN iface",
            "state": "Clean",
            "effect": "no-op",
            "evidence": "main.rs:82 refuses before any netlink call"
          }
        ],
        "forbidden": [
          "unreachable route in table 2023 after xray-up without --strict",
          "v6 pref 8999 rule without --addr6",
          "fallback metric lower than or equal to the device route metric",
          "deleting rules outside prefs 8998-9002 or routes outside table 2023"
        ],
        "seeding": [
          "All states only inside a throwaway netns via existing helpers netctl_in/ip_in (privileged.rs:28-63); device = `ip tuntap add dev nctltest0 mode tun`",
          "DeviceGone: in nctl-strict-ns add second tun nctlmain0, `ip addr add 10.99.0.1/24 dev nctlmain0`, `ip link set nctlmain0 up`, `ip route add default dev nctlmain0` (main default), xray-up --strict on nctltest0, then `ip link del nctltest0`"
        ],
        "budgets": [
          "FALLBACK_METRIC = u32::MAX = 4294967295",
          "privileged suite wall-clock 5m, --test-threads=2",
          "netlink ops unbounded inside netctl; the caller bounds the helper at 10s (helper-bounded)"
        ],
        "names": [
          "--strict",
          "Command::XrayUp.strict",
          "FALLBACK_METRIC",
          "add_fallback_route_v4",
          "add_fallback_route_v6",
          "net::xray_up(.., strict: bool)",
          "privileged::strict_up_installs_fallback_routes_and_v6_rules",
          "privileged::down_and_recover_clear_strict_state_both_families",
          "privileged::reup_across_recreated_device_leaves_one_copy",
          "privileged::strict_state_refuses_unmarked_traffic_without_device"
        ],
        "refusals": [
          "non-TUN --iface: netctl main.rs before netlink, exit 1 'refusing xray-up on <iface>: not a TUN device'",
          "bad CIDR: validate::parse_cidr before netlink"
        ]
      },
      "codeTasks": [
        "privileged.rs (tests first, own namespaces nctl-strict-ns and nctl-reup-ns, NsGuard, skip-on-no-netns like existing tests): strict_up_installs_fallback_routes_and_v6_rules (1.1: after xray-up --strict without --addr6, `ip route show table 2023` has one line starting 'unreachable default' with 'metric 4294967295'; `ip -6 route show table 2023` same; `ip -6 rule show` has 9000:, 9001:, 9002:, and 8998: when --bypass-uid given; no 8999: in v6); down_and_recover_clear_strict_state_both_families (1.2: after xray-down, and separately after recover --xray, `ip [-6] route show table 2023` empty and `ip [-6] rule show` has none of 8998:..9002:); reup_across_recreated_device_leaves_one_copy (1.3: xray-up --strict, `ip link del nctltest0`, recreate tun, xray-up --strict again; sorted lines of `ip [-6] rule show` and `ip [-6] route show table 2023` equal a fresh single run, each pref exactly once); strict_state_refuses_unmarked_traffic_without_device (1.3: see seeding; v4 `ip -4 route get 198.51.100.1` exits non-zero with 'No route to host'; `ip -4 route get 198.51.100.1 mark 255` succeeds with 'dev nctlmain0'; v6 `ip -6 route get 2001:db8::1` fails or prints 'unreachable'). Add helper ip_in_full(ns,args)->(bool,String) returning stdout+stderr.",
        "Extend up_down_is_idempotent_in_namespace: without --strict, table 2023 has no 'unreachable' and `ip -6 rule show` has no 9002: (strict-off preserves previous behavior).",
        "main.rs: Command::XrayUp gains `#[arg(long)] strict: bool` with doc line; pass to net::xray_up. XrayDown doc updated to say it also clears table 2023.",
        "net.rs: `const FALLBACK_METRIC: u32 = u32::MAX;` fns add_fallback_route_v4(handle) / add_fallback_route_v6(handle): RouteMessageBuilder destination 0/0, table_id(XRAY_ROUTE_TABLE), route type Unreachable, priority FALLBACK_METRIC, EEXIST tolerated. Confirm builder method names (kind/priority) in rtnetlink 0.21 docs before coding.",
        "net.rs xray_up signature: `xray_up(handle, iface, v4, v6, bypass_uid, capture_dns, strict: bool)`. When strict: add both fallback routes. v6 policy block runs when `v6.is_some() || strict`: add_xray_rules(Inet6) and bypass-uid v6; add_default_route_v6 and dns capture v6 only when v6.is_some().",
        "net.rs xray_down: keep del_xray_rules first, delete TUN device, then flush_table_routes(handle, XRAY_ROUTE_TABLE) (already iterates v4+v6). recover_xray unchanged (calls xray_down then flushes). Update doc comments on xray_up/xray_down.",
        "AMENDMENT: new private fns are spelled add_fallback_route_v4 / add_fallback_route_v6 (no add_unreachable_route).",
        "AMENDMENT: xray_down flushes table 2023 (flush_table_routes) right after del_xray_rules and before the device delete, so a non-ENODEV link-delete error cannot leave the unreachable routes behind.",
        "AMENDMENT: add unprivileged test main.rs tests::xray_up_parses_strict_flag — Cli::try_parse_from([\"v2ray-rs-netctl\",\"xray-up\",\"--iface\",\"xtun0\",\"--addr\",\"172.19.0.1/30\",\"--strict\"]) yields XrayUp { strict: true, .. }, and without the flag strict is false; this runs under the chunk verify. Privileged tests close only via task 5.2, which blocks archive.",
        "AMENDMENT: Command has no Debug derive, so xray_up_parses_strict_flag asserts with matches!(cli.command, Command::XrayUp { strict: true, .. })."
      ]
    },
    {
      "id": "helper-bounded",
      "tasks": [
        "2.2",
        "2.3"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. Route-helper calls get a 10s timeout, kill_on_drop, captured output into the log stream; TunRuntime carries strict and emits --strict.",
      "contract": {
        "states": [
          "HelperRunning",
          "HelperOk",
          "HelperFailed",
          "HelperTimedOut",
          "HelperCancelled"
        ],
        "transitions": [
          {
            "input": "helper exits 0",
            "state": "HelperOk",
            "effect": "set",
            "evidence": "tasks 2.2; output lines reach log stream as stderr lines"
          },
          {
            "input": "helper exits non-zero",
            "state": "HelperFailed",
            "effect": "set",
            "evidence": "tun-mode 'Route helper output is visible'; xray-up -> ProcessError::TunHelper"
          },
          {
            "input": "helper still running at HELPER_TIMEOUT",
            "state": "HelperTimedOut",
            "effect": "forced",
            "evidence": "tun-mode 'Route helper hangs': child killed, treated as failed, logged"
          },
          {
            "input": "calling future dropped (Stop wins)",
            "state": "HelperCancelled",
            "effect": "forced",
            "evidence": "design 'Cancellation safety comes from kill_on_drop'"
          },
          {
            "input": "helper spawn error (ENOENT/EACCES)",
            "state": "HelperFailed",
            "effect": "set",
            "evidence": "io error string in result, logged"
          },
          {
            "input": "xray-down fails inside stop()",
            "state": "HelperFailed",
            "effect": "no-op",
            "evidence": "tasks 2.2 'teardown_tun logs failures'; stop still reaches Stopped"
          },
          {
            "input": "TunRuntime.strict true / false",
            "state": "HelperRunning",
            "effect": "set",
            "evidence": "tasks 2.3; args contain / omit '--strict'"
          }
        ],
        "forbidden": [
          "helper with inherited stdio",
          "helper call without timeout",
          "discarded teardown result",
          "any test TunRuntime whose helper_path can resolve to the real v2ray-rs-netctl once the helper is actually invoked (use a tempdir stub script)"
        ],
        "seeding": [
          "stub helper = tempdir shell script (0o755) recording `echo \"$1\" >> calls` then the scripted behavior",
          "teardown test: in-module manager, non-TUN start of `exec sleep 30`, then in-module `mgr.tun = Some(TunRuntime{backend: Xray, iface: 'lo', helper_path: stub, strict: true, ..})` - the state a TUN start that passed preflight produces; capability probe cannot pass in tests"
        ],
        "budgets": [
          "HELPER_TIMEOUT 10s production",
          "tests pass 200ms timeout to run_helper; each test wall-clock under 3s"
        ],
        "names": [
          "TunRuntime::strict",
          "HELPER_TIMEOUT",
          "xray_up_args",
          "HelperRun",
          "run_helper",
          "ProcessManager::log_helper",
          "tun::tests::xray_up_args_include_strict_when_set",
          "tun::tests::xray_up_args_omit_strict_when_off",
          "tun::tests::helper_killed_after_timeout",
          "tun::tests::helper_output_is_captured",
          "manager::tests::teardown_failure_is_logged"
        ],
        "refusals": [
          "timeout/exit failure: process layer tun::run_helper at the deadline or exit; surfaced as ProcessError::TunHelper by the manager launch path"
        ]
      },
      "codeTasks": [
        "tun.rs tests first: xray_up_args_include_strict_when_set, xray_up_args_omit_strict_when_off (exact vec: ['xray-up','--iface','tun0','--addr','172.19.0.1/30'] then optional '--addr6' v6, '--bypass-uid' uid, '--capture-dns', '--strict' in that order); helper_killed_after_timeout (stub script writes $$ to a pid file then `exec sleep 30`; run_helper with 200ms timeout returns Err containing 'timed out', elapsed < 2s, `kill(pid, None)` then fails with ESRCH); helper_output_is_captured (stub `echo up-ok; echo 'netctl: boom' >&2; exit 1` -> output has both lines, result Err containing exit status).",
        "manager.rs test: teardown_failure_is_logged (seeding below, helper stub `exit 1`; after stop() state Stopped and log buffer has a line containing 'xray-down failed').",
        "tun.rs: `pub strict: bool` on TunRuntime (doc: install the fail-closed fallback routes; xray only). `pub(crate) const HELPER_TIMEOUT: Duration = Duration::from_secs(10);` `pub(crate) fn xray_up_args(rt: &TunRuntime) -> Vec<String>`; `pub(crate) struct HelperRun { pub output: Vec<String>, pub result: Result<(), String> }`; `pub(crate) async fn run_helper(helper: &Path, args: &[String], timeout: Duration) -> HelperRun` (stdin null, stdout/stderr piped, kill_on_drop(true), wait_with_output under tokio::time::timeout; timeout -> result Err('<verb> timed out after 10s')); xray_up(rt) and xray_down(rt) return HelperRun using HELPER_TIMEOUT.",
        "manager.rs: `fn log_helper(&self, verb: &str, run: &HelperRun)` pushes each output line as LogLine::stderr into log_buffer and emits ProcessEvent::LogLine (pattern manager.rs:192-199), plus '<verb> failed: <e>' on Err. launch path maps Err to ProcessError::TunHelper(e). teardown_tun logs instead of discarding.",
        "connection.rs build_tun_runtime: `strict: settings.tun.strict_route`. Update TunRuntime literals: connection.rs:354, manager.rs:713/799/834, tun.rs:321 (strict: false).",
        "AMENDMENT (fixes review blocker; signature unchanged: run_helper(helper: &Path, args: &[String], timeout: Duration) -> HelperRun, ProcessManager::log_helper still does the LogLine push): run_helper spawns with stdin null, stdout/stderr piped and kill_on_drop(true); two spawned reader tasks own the line buffers (outside the timed future), and only child.wait() runs inside tokio::time::timeout; on elapse it calls child.kill().await (SIGKILL + reap, no zombie), then joins the readers bounded by 500ms, and HelperRun carries the captured lines in `output` plus a failure formatted with the actual value `{timeout:?}` (not a hard-coded 10s). Production timeout const HELPER_TIMEOUT = Duration::from_secs(10). tun::tests::helper_killed_after_timeout uses a 200ms timeout and a stub that sleeps 30s, and after Err asserts nix kill(pid, None) == Err(ESRCH) immediately.",
        "AMENDMENT: while adding `strict: false` to the TunRuntime literals in manager.rs tests (helper_path PathBuf::from(\"v2ray-rs-netctl\")), switch those helper_path values to a nonexistent absolute path under the test tempdir, so no test can reach the installed helper."
      ]
    },
    {
      "id": "process-state-stop",
      "tasks": [
        "2.1"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. stop() honors every state, inspects self.child, releases TUN state, always ends Stopped; Starting->Stopping allowed.",
      "contract": {
        "states": [
          "Stopped",
          "Starting",
          "Running",
          "Stopping",
          "Error"
        ],
        "transitions": [
          {
            "input": "stop(), child Some, Running",
            "state": "Stopped",
            "effect": "set",
            "evidence": "manager.rs:271-276: Stopping, SIGTERM, 5s, SIGKILL, teardown, Stopped"
          },
          {
            "input": "stop(), child Some, Starting",
            "state": "Stopped",
            "effect": "set",
            "evidence": "process-lifecycle 'Stop during start'; needs (Starting, Stopping)"
          },
          {
            "input": "stop(), child None, Starting",
            "state": "Stopped",
            "effect": "forced",
            "evidence": "design 'stop() works from every state'"
          },
          {
            "input": "stop(), child None, Running",
            "state": "Stopped",
            "effect": "forced",
            "evidence": "design; wait future cancelled between cleanup_after_exit and handle_unexpected_exit"
          },
          {
            "input": "stop(), child None, Stopping",
            "state": "Stopped",
            "effect": "forced",
            "evidence": "idempotent completion of a cut stop"
          },
          {
            "input": "stop(), child None, Error",
            "state": "Stopped",
            "effect": "clear",
            "evidence": "process-lifecycle 'Stop while in Error with no child' + 'routing state released'; teardown then Error->Stopped (state.rs:33)"
          },
          {
            "input": "stop(), Stopped",
            "state": "Stopped",
            "effect": "no-op",
            "evidence": "process-lifecycle 'Already stopped'"
          }
        ],
        "forbidden": [
          "stop() returning with state other than Stopped",
          "Starting->Stopped direct",
          "any caller other than stop() running teardown_tun"
        ],
        "seeding": [
          "Starting without child: with_backend(SingBox), stub `[ \"$1\" = check ] && exec sleep 30; exec sleep 30`; `tokio::time::timeout(300ms, mgr.start_with_connection(None))` drops during check_config (config-check child killed by kill_on_drop)",
          "Running without child: start `exec sleep 30`; in-module `let mut c = mgr.child.take().unwrap(); c.kill().await.unwrap();`",
          "Error without child: missing binary start (existing manager.rs:737-765); tun stub attached in-module for the release assertion"
        ],
        "budgets": [
          "STOP_TIMEOUT 5s SIGTERM->SIGKILL",
          "each test under 5s wall-clock"
        ],
        "names": [
          "ProcessState::can_transition_to (Starting, Stopping)",
          "state::tests::starting_can_move_to_stopping",
          "manager::tests::stop_from_starting_without_child_reaches_stopped",
          "manager::tests::stop_from_running_without_child_reaches_stopped",
          "manager::tests::stop_from_error_releases_tun_state"
        ],
        "refusals": [
          "none new: invalid transitions still refused by ProcessState::transition with TransitionError::Invalid"
        ]
      },
      "codeTasks": [
        "Tests first. state.rs: starting_can_move_to_stopping. manager.rs: stop_from_starting_without_child_reaches_stopped, stop_from_running_without_child_reaches_stopped, stop_from_error_releases_tun_state (tun stub attached, helper calls file has exactly one 'xray-down'); each asserts subscribe() yields StateChanged to Stopping then Stopped (Error row: Stopped only) and stop() returns Ok.",
        "state.rs can_transition_to: add (Starting, Stopping).",
        "manager.rs stop(): child None and Stopped -> return Ok, no event. child None and Error -> teardown_tun, Error->Stopped. Otherwise: transition to Stopping unless already Stopping; graceful_stop (no-op without child); teardown_tun; Stopped; pid_file.remove. Remove the old early return at manager.rs:262-270.",
        "Keep shutdown() = auto_restart false + stop(). Update existing test stop_recovers_from_error_without_child only if its assertions change (they should not)."
      ]
    },
    {
      "id": "respawn-loop",
      "tasks": [
        "2.4",
        "2.5"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. Crash handling loops respawn failures into the crash budget, respawns without preflight, and never tears TUN state down outside stop().",
      "contract": {
        "states": [
          "Running",
          "RespawnWait",
          "Respawning",
          "Error",
          "Stopped"
        ],
        "transitions": [
          {
            "input": "exit while Running, auto_restart false",
            "state": "Error",
            "effect": "set",
            "evidence": "manager.rs:532-535; no helper call"
          },
          {
            "input": "exit while Running, crashes in 60s window < 3",
            "state": "RespawnWait",
            "effect": "set",
            "evidence": "process-lifecycle 'Single crash'; Running->Starting before sleep; no xray-down ('xray TUN routing state survives a respawn')"
          },
          {
            "input": "respawn succeeds",
            "state": "Running",
            "effect": "set",
            "evidence": "Starting->Running; one xray-up when tun needs helper"
          },
          {
            "input": "respawn fails (spawn io, TunDeviceTimeout, TunHelper)",
            "state": "RespawnWait",
            "effect": "no-op",
            "evidence": "process-lifecycle 'Failed respawn is retried'; state stays Starting, crash recorded"
          },
          {
            "input": "crashes in window reach 3",
            "state": "Error",
            "effect": "set",
            "evidence": "'Repeated crashes'; tasks 2.5 no helper call on give-up"
          },
          {
            "input": "Stop while RespawnWait or Respawning (future dropped)",
            "state": "Stopped",
            "effect": "forced",
            "evidence": "'Stop during a crash respawn'; via stop() rows of process-state-stop"
          }
        ],
        "forbidden": [
          "xray-down between a crash and the respawn",
          "version, capability or config check during respawn",
          "Starting->Starting transition",
          "teardown_tun on give-up or on a launch failure"
        ],
        "seeding": [
          "stub backend counts runs: `echo run >> runs; n=$(wc -l < runs); [ $n -le K ] && exit 1; exec sleep 30`",
          "tun attached in-module after a non-TUN start: `mgr.tun = Some(TunRuntime{backend: Xray, iface: 'lo', helper_path: stub, ..})`; 'lo' makes wait_for_device return at once",
          "restart_delay set in-module to 50ms"
        ],
        "budgets": [
          "CRASH_RESTART_DELAY 2s x crash count",
          "MAX_CRASHES 3",
          "CRASH_WINDOW 60s",
          "DEVICE_TIMEOUT 10s untouched",
          "tests: restart_delay 50ms, each under 5s wall-clock"
        ],
        "names": [
          "ProcessManager::launch",
          "ProcessManager::respawn",
          "ProcessManager.restart_delay",
          "manager::tests::respawn_skips_preflight",
          "manager::tests::failed_respawn_is_retried",
          "manager::tests::respawn_budget_exhaustion_errors",
          "manager::tests::stop_during_respawn_wait_reaches_stopped"
        ],
        "refusals": [
          "give-up: process layer handle_unexpected_exit when the window holds MAX_CRASHES, Error message contains '3 crashes within 60s'"
        ]
      },
      "codeTasks": [
        "Tests first (manager.rs, restart_delay 50ms): respawn_skips_preflight (SingBox check stub logs 'check' to a file; backend run 1 exits 1, run 2 sleeps; after wait_and_handle_exit: Running, checks=1, runs=2); failed_respawn_is_retried (tun stub on 'lo', helper fails first xray-up and succeeds second; ends Running; calls = xray-up,xray-up; no xray-down); respawn_budget_exhaustion_errors (backend always exit 1 after first spawn; loop `while mgr.state() == Running { mgr.wait_and_handle_exit().await }`; Error containing '3 crashes'; calls has zero 'xray-down'); stop_during_respawn_wait_reaches_stopped (restart_delay 5s; timeout(300ms, wait_and_handle_exit) then shutdown(); Stopped, calls exactly one 'xray-down', zero 'xray-up').",
        "manager.rs: field `restart_delay: Duration` initialized to CRASH_RESTART_DELAY in new().",
        "Rename spawn_process -> `async fn launch(&mut self) -> Result<(), ProcessError>`: on device timeout or xray-up failure run graceful_stop only, no teardown_tun (drop manager.rs:343,350,355 teardown calls).",
        "Add `async fn respawn(&mut self) -> Result<(), ProcessError>`: `self.launch().await?` then Starting->Running with current_connection. Reuses binary_path, config_path, geodata_dir, tun, current_connection unchanged since the start; skips xray_version_triple, has_net_admin, check_config.",
        "handle_unexpected_exit: record crash; if !auto_restart -> Error(msg), return. Loop: if crash_times.len() >= MAX_CRASHES -> Error('{MAX_CRASHES} crashes within {CRASH_WINDOW:?}: {msg}'), return; if state is Running -> transition Starting(current_connection) (never Starting->Starting); sleep(restart_delay * crashes); respawn(): Ok -> return; Err(e) -> log 'restart failed: {e}', msg = that, record crash, continue. Remove the pre-respawn teardown_tun and its comment (manager.rs:527-530).",
        "AMENDMENT: respawn_skips_preflight also asserts a fresh ProcessManager::new(..) has restart delay == CRASH_RESTART_DELAY (the 2s spec default), since the other respawn tests override it to 50ms."
      ]
    },
    {
      "id": "conn-terminal-state",
      "tasks": [
        "3.1",
        "3.2"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. Only the connection task reports Stopped/Error; Stop wins during start; marker written from TunRuntime before start.",
      "contract": {
        "states": [
          "Starting",
          "Running",
          "Stopping",
          "Stopped",
          "Error"
        ],
        "transitions": [
          {
            "input": "manager StateChanged to Starting|Running|Stopping",
            "state": "Running",
            "effect": "set",
            "evidence": "design 'Terminal states come only from the supervising task'; relayed with the generation"
          },
          {
            "input": "manager StateChanged to Stopped|Error",
            "state": "Running",
            "effect": "no-op",
            "evidence": "connection.rs:210 extended to Stopped"
          },
          {
            "input": "candidate gives up, more candidates remain",
            "state": "Starting",
            "effect": "no-op",
            "evidence": "'Failover is not reported as a stop': no Stopped, handle and marker kept"
          },
          {
            "input": "Stop at any point (queued, pre-start, during start, supervising, after failover)",
            "state": "Stopped",
            "effect": "forced",
            "evidence": "tasks 3.1; exactly one Stopped"
          },
          {
            "input": "last candidate fails",
            "state": "Error",
            "effect": "set",
            "evidence": "'Last candidate fails': one Error(summarize_failures)"
          },
          {
            "input": "TunRuntime Some before start",
            "state": "Starting",
            "effect": "set",
            "evidence": "tun-mode 'Marker precedes route changes'; TunSession{backend: rt.backend, iface: rt.iface}"
          }
        ],
        "forbidden": [
          "two terminal messages for one generation",
          "any message for the generation after its terminal one",
          "Stopped on failover",
          "marker written after start_with_connection or from settings"
        ],
        "seeding": [
          "AppPaths::for_profile_in(AppProfile::Test, tmp); ConfigWriter::new(&settings, &paths); settings.backend.binary_path unused (request.binary_path = stub)",
          "candidates: Shadowsocks 203.0.113.1 and 203.0.113.2 (no DNS pin lookup)",
          "stub backend: `[ \"$1\" = check ] && exit 0; grep -q 203.0.113.1 \"$3\" && exit 1; exec sleep 30`, backend SingBox, TUN off",
          "marker test: tun.enabled, backend Xray, stub without caps -> start fails at the capability gate after the marker write"
        ],
        "budgets": [
          "crash-looping candidate costs 2s+4s real (restart_delay not exposed across crates, by choice); per-message recv timeout 20s; test wall-clock under 30s"
        ],
        "names": [
          "relays",
          "tun_session_for",
          "ConnectionRequest.paths",
          "connection::tests::forwarder_relays_only_nonterminal_states",
          "connection::tests::failover_reports_no_stopped_and_stop_reports_one",
          "connection::tests::last_candidate_failure_reports_one_error",
          "connection::tests::marker_written_from_runtime_before_start"
        ],
        "refusals": [
          "Stop: connection task select at the first await after the command; manager stop() does the release"
        ]
      },
      "codeTasks": [
        "Tests first (connection.rs, #[tokio::test(flavor = 'multi_thread')]): forwarder_relays_only_nonterminal_states (pure); failover_reports_no_stopped_and_stop_reports_one; last_candidate_failure_reports_one_error; marker_written_from_runtime_before_start. No manager trait: the seam is connection::spawn with a stub backend script and `relm4::channel::<AppMsg>()` (re-exported by relm4 0.10; confirm) as the sender.",
        "`fn relays(state: &ProcessState) -> bool` = Starting|Running|Stopping; forwarder uses it; keep its JoinHandle.",
        "Before every terminal emit and before failing over: `forwarder.abort(); let _ = forwarder.await;` so no relayed state follows the terminal one.",
        "Failover and start-failure paths drop shutdown(): keep the failed manager as `parked: Option<ProcessManager>` (replaced per candidate, dropped at final Error). Stop at loop top or pre-start: `if let Some(mut m) = parked.take() { m.shutdown().await }` then emit Stopped, so leftover routes are released.",
        "Wrap start_with_connection in `tokio::select! { biased; Some(ConnectionCmd::Stop) = cmd_rx.recv() => { mgr.shutdown().await; emit Stopped; return } r = mgr.start_with_connection(..) => r }`. Supervise loop Stop branch: shutdown, abort forwarder, emit Stopped, return.",
        "ConnectionRequest gains `pub paths: AppPaths`; app.rs start_connection passes self.paths.clone(). `fn tun_session_for(rt: &TunRuntime) -> TunSession`. Per candidate, when the runtime is Some, save_tun_session before start_with_connection; failure -> log::warn. Delete the marker write at app.rs:1128-1142.",
        "AMENDMENT: marker_written_from_runtime_before_start uses a single candidate and a stub that answers `version` at once (`[ \"$1\" = version ] && echo \"Xray 26.6.27\" && exit 0`), so it never waits CONFIG_CHECK_TIMEOUT.",
        "AMENDMENT: parking a failed manager aborts its log-forwarder task together with its state forwarder, so a failed candidate cannot emit log lines after the next candidate starts."
      ]
    },
    {
      "id": "app-killswitch",
      "tasks": [
        "3.3",
        "3.4"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. App releases the kill-switch through the marker-driven recovery pass exactly when it stops trying; Quit waits for Stopped or the release.",
      "contract": {
        "states": [
          "Idle",
          "Live",
          "StopInFlight",
          "Held",
          "Releasing",
          "Exiting"
        ],
        "transitions": [
          {
            "input": "Error, retry follows",
            "state": "Held",
            "effect": "no-op",
            "evidence": "tun-mode 'Blocked across automatic reconnects'"
          },
          {
            "input": "Error, attempts >= 3 or no retry path",
            "state": "Releasing",
            "effect": "set",
            "evidence": "'Released on final give-up'; state stays Error"
          },
          {
            "input": "Error while app state was Stopping",
            "state": "Releasing",
            "effect": "set",
            "evidence": "user already disconnected; no auto-reconnect"
          },
          {
            "input": "Disconnect, no handle, marker present",
            "state": "Releasing",
            "effect": "set",
            "evidence": "'Released on Disconnect without a live backend'; reports Stopped"
          },
          {
            "input": "Disconnect, no handle, no marker",
            "state": "Idle",
            "effect": "no-op",
            "evidence": "app.rs:1090-1093"
          },
          {
            "input": "Quit, handle Some",
            "state": "Exiting",
            "effect": "set",
            "evidence": "app.rs:1211-1213; destroy on Stopped"
          },
          {
            "input": "Quit, no handle, app state Stopping",
            "state": "Exiting",
            "effect": "forced",
            "evidence": "'Quit during a stop'; tasks 3.4; today app.rs:1214 destroys at once"
          },
          {
            "input": "Quit, no handle, marker present",
            "state": "Releasing",
            "effect": "set",
            "evidence": "tun-mode 'released on Quit'; destroy on TunReleased"
          },
          {
            "input": "Quit idle",
            "state": "Idle",
            "effect": "clear",
            "evidence": "window.destroy"
          },
          {
            "input": "TunReleased",
            "state": "Idle",
            "effect": "clear",
            "evidence": "marker cleared by recover_tun_session; destroy if pending_exit"
          }
        ],
        "forbidden": [
          "window destroyed while app state Stopping or a release in flight",
          "marker cleared while a retry is scheduled",
          "Connect starting while tun_release_in_flight",
          "release while a handle is Some"
        ],
        "seeding": [
          "pure fns take plain args",
          "recover tests: AppPaths::for_profile_in(AppProfile::Test, tmp) + save_tun_session(TunSession{backend: Xray, iface: 'tun9'}) + tempdir stub helper"
        ],
        "budgets": [
          "RECOVER_TIMEOUT 5s",
          "MAX_AUTO_RECONNECTS 3",
          "AUTO_RECONNECT_DELAY 5s",
          "each test under 2s"
        ],
        "names": [
          "QuitPlan",
          "quit_plan",
          "auto_reconnect_allowed",
          "release_tun_session",
          "tun_release_in_flight",
          "AppMsg::TunReleased",
          "recover_tun_session(paths, helper)",
          "app::tests::quit_while_stopping_waits_for_stopped",
          "app::tests::quit_with_handle_stops_first",
          "app::tests::quit_with_leftover_marker_releases_first",
          "app::tests::quit_idle_exits",
          "app::tests::auto_reconnect_exhausted_after_three_attempts",
          "app::tests::auto_reconnect_suppressed_on_exit",
          "app::tests::recover_runs_helper_and_clears_marker",
          "app::tests::recover_clears_marker_when_helper_fails",
          "disconnect_plan",
          "DisconnectPlan { Stop, Release, Nothing }",
          "release_on_error"
        ],
        "refusals": [
          "helper hang: run_with_timeout kills at RECOVER_TIMEOUT, marker cleared regardless (app.rs:1590-1598)"
        ]
      },
      "codeTasks": [
        "Tests first (app.rs pure fns): quit_while_stopping_waits_for_stopped, quit_with_handle_stops_first, quit_with_leftover_marker_releases_first, quit_idle_exits, auto_reconnect_exhausted_after_three_attempts, auto_reconnect_suppressed_on_exit, recover_runs_helper_and_clears_marker (stub helper writes \"$@\"; args exactly 'recover --xray --iface tun9'), recover_clears_marker_when_helper_fails (stub exit 1).",
        "`recover_tun_session(paths: &AppPaths, helper: &Path)`; init caller passes &v2ray_rs_process::helper_path(). Tests never run the real helper.",
        "`enum QuitPlan { Stop, AwaitStopped, Release, Exit }`, `fn quit_plan(has_handle: bool, state: &ProcessState, marker_present: bool) -> QuitPlan`; TrayQuit and CloseRequested (non-tray branch) use it; AwaitStopped and Release set pending_exit.",
        "`fn auto_reconnect_allowed(pending_exit: bool, attempts: u32) -> bool`; schedule_auto_reconnect returns bool.",
        "`fn release_tun_session(&mut self, sender)`: sets `tun_release_in_flight`, tokio::spawn { lifecycle.lock(); spawn_blocking(recover_tun_session) } then `AppMsg::TunReleased`. Handler clears the flag; destroys window if pending_exit. Connect and ConnectToNode return early while the flag is set.",
        "ProcessStateConnection Error: release when no retry follows (no reconnect_pending, no replay target, auto_reconnect_allowed false or not scheduled, direct_connect_in_flight, pending_exit, or app state was Stopping). Disconnect without handle and marker present: cancel_auto_reconnect, apply_state(Stopped), release; else existing 'Not connected' toast.",
        "AMENDMENT: extract pure fns beside quit_plan: disconnect_plan(has_handle: bool, marker_present: bool) -> DisconnectPlan { Stop, Release, Nothing } and release_on_error(app_state_stopping: bool, reconnects_left: u32) -> bool — the retry-budget sub-decision only; the Error handler still combines it with reconnect_pending, the replay target, direct_connect_in_flight and pending_exit as today. The Stopped report after a Release rests on apply_state(Stopped) in the handler (accepted gap, no handler-level test). Tests app::tests::disconnect_without_handle_releases_marker and app::tests::error_while_stopping_releases_without_retry, plus app::tests::error_with_reconnects_left_keeps_killswitch."
      ]
    },
    {
      "id": "cancel-starting",
      "tasks": [
        "3.5"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. Window button and tray item show an enabled Disconnect during Starting; the existing Disconnect reaches the task select.",
      "contract": {
        "states": [
          "Stopped",
          "Starting",
          "Running",
          "Stopping",
          "Error"
        ],
        "transitions": [
          {
            "input": "state Starting (incl. respawn)",
            "state": "Starting",
            "effect": "set",
            "evidence": "ui-statusbar-logs 'Starting state button is actionable'; system-tray 'Disconnect enabled while starting'"
          },
          {
            "input": "state Stopping",
            "state": "Stopping",
            "effect": "set",
            "evidence": "system-tray 'disabled during transitions'"
          },
          {
            "input": "Disconnect during Starting",
            "state": "Stopped",
            "effect": "forced",
            "evidence": "'Cancel a slow connect'; conn-terminal-state select"
          }
        ],
        "forbidden": [
          "Connect offered while Starting",
          "enabled item while Stopping"
        ],
        "seeding": [
          "pure fns take a ProcessState"
        ],
        "budgets": [
          "tests under 1s"
        ],
        "names": [
          "connect_toggle",
          "toggle_action"
        ],
        "refusals": [
          "none"
        ]
      },
      "codeTasks": [
        "Tests first: app::tests::toggle_is_actionable_disconnect_while_starting, app::tests::toggle_disabled_while_stopping, tray::tests::toggle_is_enabled_disconnect_while_starting, tray::tests::toggle_disabled_while_stopping (TrayAction has no PartialEq: use matches!).",
        "app.rs `fn connect_toggle(state: &ProcessState) -> (bool, bool)` = (shows_disconnect, sensitive): Stopped (false,true), Starting (true,true), Running (true,true), Stopping (true,false), Error (false,true); apply_state sets connected/button_sensitive from it. `connected` is read only by the button and ToggleConnection.",
        "tray.rs `fn toggle_action(state: &ProcessState) -> (TrayAction, bool)`: Running|Starting (Disconnect,true), Stopping (Connect,false), Stopped|Error (Connect,true); menu() builds the item from it."
      ]
    },
    {
      "id": "prefs-docs",
      "tasks": [
        "4.1",
        "4.2"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. strict_route row sensitive for xray with a note on what it blocks; ARCHITECTURE and CHANGELOG record the new crash path and IPv6 break.",
      "contract": {
        "states": [
          "StrictSensitive",
          "StrictInsensitive"
        ],
        "transitions": [
          {
            "input": "backend Xray or SingBox",
            "state": "StrictSensitive",
            "effect": "set",
            "evidence": "proposal 'strict_route applies to xray'"
          },
          {
            "input": "backend V2ray",
            "state": "StrictInsensitive",
            "effect": "set",
            "evidence": "no TUN for v2ray"
          }
        ],
        "forbidden": [
          "note claiming strict is sing-box only"
        ],
        "seeding": [
          "pure fn"
        ],
        "budgets": [
          "test under 1s"
        ],
        "names": [
          "strict_route_applies",
          "preferences::tun::tests::strict_route_row_sensitive_for_xray"
        ],
        "refusals": [
          "none"
        ]
      },
      "codeTasks": [
        "preferences/tun.rs new test module: strict_route_row_sensitive_for_xray (strict_route_applies(Xray) and (SingBox) true, (V2ray) false).",
        "`fn strict_route_applies(backend: BackendType) -> bool`; strict_row.set_sensitive uses it at tun.rs:725 and :736; strict row subtitle states it applies to both backends and, under xray, blocks traffic while reconnecting and IPv6 without an IPv6 tunnel address; advanced_note (tun.rs:126-130) narrowed so it no longer claims strict is sing-box only.",
        "docs/ARCHITECTURE.md: state machine (104-106: Starting->Stopping), crash recovery (108-118: respawn loop, preflight reuse, no teardown), netctl (148-165: --strict, v6 rules, xray-down clears table 2023), stop/marker (229-238: teardown only from stop, release owned by the app, marker written by the connection task before start).",
        "CHANGELOG.md [Unreleased] Changed: BREAKING for xray TUN with strict_route on (default): IPv6 refused without an IPv6 tunnel address; host blocked while reconnecting. Fixed: Disconnect during respawn/start, failover handle loss, helper timeout."
      ]
    },
    {
      "id": "verification",
      "tasks": [
        "5.1",
        "5.2",
        "5.3",
        "5.4"
      ],
      "summary": "NO-RED-WAIVER: manual live check; NO-TESTER-WAIVER: manual live check, run by the user after merge. 5.1 is the automated floor run by the pipeline; 5.2 needs root and 5.3/5.4 need a live xray host: leave them unchecked in tasks.md with a hand-off note and do not archive until the user reports them.",
      "contract": {
        "states": [
          "FloorGreen",
          "HandedOff"
        ],
        "transitions": [
          {
            "input": "floor passes",
            "state": "FloorGreen",
            "effect": "set",
            "evidence": "tasks 5.1"
          },
          {
            "input": "privileged or live check pending",
            "state": "HandedOff",
            "effect": "no-op",
            "evidence": "tasks 5.2-5.4 need root / a live host"
          }
        ],
        "forbidden": [
          "marking 5.2-5.4 done without a user report"
        ],
        "seeding": [
          "none"
        ],
        "budgets": [
          "floor 10m, 4 threads",
          "privileged 5m"
        ],
        "names": [],
        "refusals": [
          "none"
        ]
      },
      "codeTasks": [
        "5.1: make lint && make test TEST_TIMEOUT=10m",
        "5.2 (user, root): CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' timeout 5m cargo test -p v2ray-rs-netctl --features privileged-tests -- --test-threads=2 (builds as the user, runs the test binary under sudo; avoids a root-owned target/)",
        "5.3 (user): kill -SEGV xray during a curl loop; expect host-unreachable, no success via the real interface (ip route get, ss -tnp), recovery",
        "5.4 (user): Disconnect in the respawn window; UI Disconnected, ip rule without 8998-9002, table 2023 empty for -4 and -6"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "### Requirement: Stop backend process The system SHALL gracefully stop the running backend process using SIGTERM, falling back to SIGKILL after a timeout. A stop request SHALL be honored from every process state — including while a crash respawn is waiting or starting, while a connection is starting, and while a route-helper call is in flight — and SHALL always end in a reported `Stopped` state with any TUN routing state released.",
      "tests": [
        "state::tests::starting_can_move_to_stopping",
        "manager::tests::stop_from_starting_without_child_reaches_stopped",
        "manager::tests::stop_from_running_without_child_reaches_stopped",
        "manager::tests::stop_from_error_releases_tun_state",
        "manager::tests::stop_during_respawn_wait_reaches_stopped"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL send SIGTERM, wait up to 5 seconds for exit, then send SIGKILL if still running",
      "tests": [
        "lifecycle::start_and_stop"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL return `Ok(())` silently and remain in Stopped state",
      "tests": [
        "lifecycle::stop_when_already_stopped"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL transition to `Stopped` and return `Ok(())` instead of leaving the manager parked in `Error`",
      "tests": [
        "manager::tests::stop_recovers_from_error_without_child",
        "manager::tests::stop_from_error_releases_tun_state"
      ]
    },
    {
      "shall": "- **THEN** the respawn SHALL be abandoned, any backend it spawned SHALL be stopped, TUN routing state SHALL be released, and the system SHALL report `Stopped`",
      "tests": [
        "manager::tests::stop_during_respawn_wait_reaches_stopped"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL stop any spawned backend, cancel any in-flight route-helper call, release TUN routing state, and report `Stopped`",
      "tests": [
        "manager::tests::stop_from_starting_without_child_reaches_stopped",
        "tun::tests::helper_killed_after_timeout"
      ]
    },
    {
      "shall": "- **THEN** the application SHALL wait for the stop to report `Stopped` before exiting, so TUN teardown is not cut short",
      "tests": [
        "app::tests::quit_while_stopping_waits_for_stopped"
      ]
    },
    {
      "shall": "### Requirement: Crash detection and recovery The system SHALL detect unexpected process exits and restart automatically within a bounded budget. A respawn that fails to start SHALL count as a crash and SHALL be retried while the budget allows. An in-place respawn SHALL reuse the pre-launch validation of the start it replaces, relaunching the same binary with the same config without re-probing the version, the capabilities, or the config. For xray in TUN mode the respawn SHALL NOT remove the session's routing state.",
      "tests": [
        "manager::tests::respawn_skips_preflight",
        "manager::tests::failed_respawn_is_retried",
        "manager::tests::respawn_budget_exhaustion_errors",
        "manager::tests::stop_during_respawn_wait_reaches_stopped"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL wait 2 seconds and attempt to restart automatically",
      "tests": [
        "lifecycle::crash_detection",
        "manager::tests::respawn_skips_preflight"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL transition to Error state instead of restarting. Any exit while the backend is expected to be running counts as a crash, including a signal death (OOM, segfault, external kill); a requested stop moves the state to `Stopping` first and never reaches crash handling.",
      "tests": [
        "lifecycle::signal_death_is_treated_as_crash",
        "manager::tests::respawn_budget_exhaustion_errors"
      ]
    },
    {
      "shall": "- **THEN** the failure SHALL be recorded as a crash and the system SHALL wait and respawn again instead of transitioning to Error",
      "tests": [
        "manager::tests::failed_respawn_is_retried"
      ]
    },
    {
      "shall": "- **THEN** it SHALL relaunch without re-running the backend version probe, the capability probe, or the backend's config check",
      "tests": [
        "manager::tests::respawn_skips_preflight"
      ]
    },
    {
      "shall": "- **THEN** the policy rules installed for the session SHALL remain in place from the exit until the respawned backend's routes are programmed",
      "tests": [
        "manager::tests::failed_respawn_is_retried",
        "manual:5.3"
      ]
    },
    {
      "shall": "### Requirement: TUN-aware connection start and stop The system SHALL make connection start and stop TUN-aware. For xray with TUN enabled, after spawning the backend the system SHALL wait for the TUN device to appear, bounded by a timeout, then invoke the route helper to program the address and routes before reporting `Running`; if the device does not appear or the helper fails, the system SHALL stop the backend and transition to `Error`. For sing-box with TUN enabled, the backend programs its own routes via `auto_route` and the system SHALL NOT run the route helper. Stop SHALL remain SIGTERM-first so the backend can tear down its own routes before any SIGKILL. Every route-helper invocation SHALL be bounded by a timeout and terminated if its caller is cancelled, and its output SHALL reach the process log stream. The TUN recovery marker SHALL be persisted before the route helper first changes routes, naming the backend and the interface the session actually uses.",
      "tests": [
        "tun::tests::helper_killed_after_timeout",
        "tun::tests::helper_output_is_captured",
        "connection::tests::marker_written_from_runtime_before_start"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL spawn xray, wait for the TUN device, invoke the route helper to add the split routes, and only then report `Running`",
      "tests": [
        "connection::tests::marker_written_from_runtime_before_start",
        "manual:5.3"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL stop the backend and transition to `Error`",
      "tests": [
        "tun::tests::wait_for_missing_device_times_out"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL spawn sing-box and rely on its `auto_route` to program routes, without invoking the route helper",
      "tests": [
        "tun::tests::xray_needs_helper_singbox_does_not"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL send SIGTERM first so the backend can remove its own routes (sing-box) or close its TUN fd so the kernel drops the routes (xray), escalating to SIGKILL only after the timeout, and for xray SHALL invoke the route-helper teardown as a safeguard",
      "tests": [
        "lifecycle::start_and_stop",
        "manual:5.4"
      ]
    },
    {
      "shall": "- **THEN** the helper process SHALL be killed, the invocation SHALL be treated as failed, and the failure SHALL appear in the process log stream",
      "tests": [
        "tun::tests::helper_killed_after_timeout"
      ]
    },
    {
      "shall": "- **THEN** that output or failure SHALL appear in the process log stream",
      "tests": [
        "tun::tests::helper_output_is_captured",
        "manager::tests::teardown_failure_is_logged"
      ]
    },
    {
      "shall": "- **THEN** the TUN recovery marker SHALL be persisted, with the runtime's interface name, before the route helper is first invoked",
      "tests": [
        "connection::tests::marker_written_from_runtime_before_start"
      ]
    },
    {
      "shall": "### Requirement: Connection terminal state has a single source The system SHALL report a connection's terminal state (`Stopped` or `Error`) only from the component supervising the whole connection attempt, never from an individual candidate's backend. A candidate given up during failover SHALL NOT be reported as the connection stopping.",
      "tests": [
        "connection::tests::forwarder_relays_only_nonterminal_states",
        "connection::tests::failover_reports_no_stopped_and_stop_reports_one"
      ]
    },
    {
      "shall": "- **THEN** the app SHALL NOT receive `Stopped` for the connection, SHALL keep its connection handle, and SHALL keep the TUN recovery marker",
      "tests": [
        "connection::tests::failover_reports_no_stopped_and_stop_reports_one"
      ]
    },
    {
      "shall": "- **THEN** Disconnect SHALL stop that backend and report `Stopped`",
      "tests": [
        "connection::tests::failover_reports_no_stopped_and_stop_reports_one"
      ]
    },
    {
      "shall": "- **THEN** the app SHALL receive exactly one terminal `Error` summarizing the failures",
      "tests": [
        "connection::tests::last_candidate_failure_reports_one_error"
      ]
    },
    {
      "shall": "### Requirement: In-flight connection can be cancelled The system SHALL let the user cancel a connection that is starting, including an in-place crash respawn, from the main window and from the tray.",
      "tests": [
        "app::tests::toggle_is_actionable_disconnect_while_starting",
        "tray::tests::toggle_is_enabled_disconnect_while_starting"
      ]
    },
    {
      "shall": "- **THEN** the attempt SHALL be abandoned and the connection SHALL end in `Stopped`",
      "tests": [
        "manager::tests::stop_from_starting_without_child_reaches_stopped"
      ]
    },
    {
      "shall": "### Requirement: Privileged route helper for xray The system SHALL include a minimal privileged helper binary that programs and removes the xray TUN routing state, because xray does not configure system routes on Linux. The helper SHALL be idempotent. `xray-up` SHALL ensure the link is up, assign the address(es) ignoring an already-present address, install a default route bound to the TUN device in a dedicated routing table (2023), and install policy rules: fwmark-255 traffic looks up `main` (pref 9000), unmarked traffic looks up `main` with the default route suppressed (`suppress_prefixlength 0`, pref 9001), and everything else looks up the TUN table (pref 9002); with `--bypass-uid`, a uid-range rule to `main` at pref 8998; with `--capture-dns`, unmarked udp and tcp traffic to port 53 looks up the TUN table (pref 8999), so a resolver on the local subnet is reached through the tunnel rather than the LAN route pref 9001 preserves. IPv6 equivalents SHALL be installed when an IPv6 address is supplied. With `--strict`, `xray-up` SHALL additionally install an `unreachable` default route with the lowest priority in table 2023 for IPv4 and IPv6, and SHALL install the IPv6 policy rules even when no IPv6 address is supplied, so that traffic destined for the tunnel is refused rather than routed through the real default route whenever the TUN device is absent. Re-running `xray-up` against a recreated device of the same name SHALL succeed and leave exactly one copy of each route and rule.",
      "tests": [
        "privileged::strict_up_installs_fallback_routes_and_v6_rules",
        "privileged::down_and_recover_clear_strict_state_both_families",
        "privileged::reup_across_recreated_device_leaves_one_copy",
        "privileged::strict_state_refuses_unmarked_traffic_without_device",
        "privileged::up_down_is_idempotent_in_namespace",
        "main::tests::xray_up_parses_strict_flag"
      ]
    },
    {
      "shall": "- **THEN** the helper SHALL bring the link up, assign the address(es), install the table-2023 default route bound to the device, and install the pref 9000/9001/9002 policy rules (plus the pref 8998 uid-range rule when `--bypass-uid` is given and the pref 8999 port-53 rules when `--capture-dns` is given), each step idempotent",
      "tests": [
        "privileged::up_down_is_idempotent_in_namespace"
      ]
    },
    {
      "shall": "- **THEN** they SHALL match only unmarked traffic, so the backend's own resolver queries keep egressing the real interface through the pref 9000 rule",
      "tests": [
        "privileged::capture_dns_steers_port_53_into_the_tunnel_table"
      ]
    },
    {
      "shall": "- **THEN** table 2023 SHALL contain an `unreachable` default route for IPv4 and for IPv6 at a lower priority than the device route, and the IPv6 pref 9000/9001/9002 rules SHALL be present whether or not an IPv6 address was supplied",
      "tests": [
        "privileged::strict_up_installs_fallback_routes_and_v6_rules"
      ]
    },
    {
      "shall": "- **THEN** unmarked traffic to a destination outside the on-link routes SHALL be refused with host-unreachable rather than sent through the real default route, while fwmark-255 and bypass-uid traffic SHALL keep using `main`",
      "tests": [
        "privileged::strict_state_refuses_unmarked_traffic_without_device"
      ]
    },
    {
      "shall": "- **THEN** the helper SHALL succeed and the resulting routes and rules SHALL match a single fresh `xray-up`",
      "tests": [
        "privileged::reup_across_recreated_device_leaves_one_copy"
      ]
    },
    {
      "shall": "- **THEN** the helper SHALL remove the policy rules it owns (matching its reserved preferences) for both address families, remove the table-2023 fallback routes, delete the device only if it is a TUN device — removing its addresses and device-scoped routes — and SHALL succeed as a no-op when all are already absent",
      "tests": [
        "privileged::down_and_recover_clear_strict_state_both_families"
      ]
    },
    {
      "shall": "- **THEN** the helper SHALL remove any leftover TUN device and its policy rules, flush its dedicated routing table (2023 for xray) including the fallback routes, and for sing-box additionally flush the routing rules and table its `auto_route` uses, leaving system networking clean",
      "tests": [
        "privileged::down_and_recover_clear_strict_state_both_families"
      ]
    },
    {
      "shall": "### Requirement: Traffic stays blocked while an xray TUN session reconnects When the backend is xray, TUN is enabled and `strict_route` is on, the system SHALL keep the session's strict routing state installed from the moment the backend exits unexpectedly until the session is running again or the system stops trying — across in-place respawns, failover to another candidate, and automatic reconnect attempts. The system SHALL release it, restoring the host's normal routing, on Disconnect, on Quit, and when automatic reconnects are exhausted. With `strict_route` off, the system SHALL install no fallback routes and no IPv6 rules without an IPv6 address, preserving the previous behavior.",
      "tests": [
        "manager::tests::failed_respawn_is_retried",
        "app::tests::auto_reconnect_exhausted_after_three_attempts",
        "manual:5.3"
      ]
    },
    {
      "shall": "- **THEN** no unmarked traffic outside the on-link routes SHALL leave through the real default route until the respawned backend's routes are up",
      "tests": [
        "privileged::strict_state_refuses_unmarked_traffic_without_device",
        "manual:5.3"
      ]
    },
    {
      "shall": "- **THEN** the strict routing state SHALL remain installed until that reconnect succeeds or the automatic reconnects are exhausted",
      "tests": [
        "app::tests::auto_reconnect_exhausted_after_three_attempts",
        "app::tests::error_with_reconnects_left_keeps_killswitch"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL remove the session's routing state and the TUN recovery marker, restoring normal connectivity",
      "tests": [
        "app::tests::recover_runs_helper_and_clears_marker",
        "app::tests::error_while_stopping_releases_without_retry"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL remove the routing state and the marker and report `Stopped`",
      "tests": [
        "app::tests::disconnect_without_handle_releases_marker",
        "manager::tests::stop_from_error_releases_tun_state"
      ]
    },
    {
      "shall": "- **THEN** IPv6 traffic to destinations outside the on-link routes SHALL be refused rather than sent through the real default route",
      "tests": [
        "privileged::strict_up_installs_fallback_routes_and_v6_rules"
      ]
    },
    {
      "shall": "- **THEN** the route helper SHALL be invoked without `--strict`, and IPv6 SHALL be routed into the tunnel only when an IPv6 tunnel address is set",
      "tests": [
        "tun::tests::xray_up_args_omit_strict_when_off",
        "privileged::up_down_is_idempotent_in_namespace"
      ]
    },
    {
      "shall": "### Requirement: Tray context menu The system SHALL display a context menu when the tray icon is activated.",
      "tests": [
        "tray::tests::toggle_disabled_while_stopping"
      ]
    },
    {
      "shall": "- **THEN** the menu SHALL show: \"Connect\", separator, status label (\"Status: Disconnected\", disabled), separator, \"Open Main Window\", \"Quit\"",
      "tests": [
        "tray::tests::toggle_disabled_while_stopping"
      ]
    },
    {
      "shall": "- **THEN** the menu SHALL show: \"Disconnect\", separator, status label (\"Status: Connected (node name)\", disabled), separator, \"Open Main Window\", \"Quit\"",
      "tests": [
        "tray::tests::connected_status_text_includes_source_and_node"
      ]
    },
    {
      "shall": "- **THEN** the menu SHALL show an enabled \"Disconnect\" item that cancels the connection attempt",
      "tests": [
        "tray::tests::toggle_is_enabled_disconnect_while_starting"
      ]
    },
    {
      "shall": "- **THEN** the Connect/Disconnect menu item SHALL be disabled (not clickable)",
      "tests": [
        "tray::tests::toggle_disabled_while_stopping"
      ]
    },
    {
      "shall": "### Requirement: Connect button has icon and label The connect/disconnect button SHALL display both a symbolic icon and a text label.",
      "tests": [
        "app::tests::toggle_is_actionable_disconnect_while_starting"
      ]
    }
  ],
  "testHarness": [
    "write_script — crates/process/src/manager.rs:633 — writes `#!/bin/sh` + body as `backend` (0755) in a dir",
    "manager_for — crates/process/src/manager.rs:641 — ProcessManager over write_script stub, `{}` config.json, backend.pid, no geodata",
    "fake-xray version stub (inline) — crates/process/src/manager.rs:788,823 — script echoing `Xray X.Y.Z` for the version probe, paired with TunRuntime{Xray, helper_path \"v2ray-rs-netctl\"}",
    "TunRuntime test literal — crates/process/src/manager.rs:713 — Xray tun0 172.19.0.1/30 with /bin/sh as capability-less backend",
    "setup_dir — crates/process/tests/lifecycle.rs:9 — TempDir",
    "create_script — crates/process/tests/lifecycle.rs:13 — named 0755 script with sync_all (ETXTBSY-safe)",
    "create_config — crates/process/tests/lifecycle.rs:23 — `{}` config.json",
    "pid_path — crates/process/tests/lifecycle.rs:29 — test.pid path",
    "mk closure — crates/process/src/tun.rs:321 — TunRuntime per BackendType",
    "t — crates/process/src/tun.rs:248 — SystemTime from epoch secs",
    "consts BIN/NS/NS_DNS/IFACE/ADDR/ADDR6 — crates/netctl/tests/privileged.rs:10-17 — netctl bin path, netns names, nctltest0, 172.31.255.1/30, fd00:ffff::1/64",
    "run — crates/netctl/tests/privileged.rs:19 — run command, bool success",
    "ip_in / ip_in_ns — crates/netctl/tests/privileged.rs:28,34 — `ip netns exec <ns> ip ...` status",
    "ip_in_output / ip_in_ns_output — crates/netctl/tests/privileged.rs:39,49 — same, stdout string",
    "netctl_in / netctl — crates/netctl/tests/privileged.rs:55,61 — run netctl binary inside netns",
    "device_exists — crates/netctl/tests/privileged.rs:66 — `ip link show nctltest0` in NS",
    "NsGuard — crates/netctl/tests/privileged.rs:72 — deletes netns on drop",
    "node — crates/ui/src/connection.rs:396 — Shadowsocks ProxyNode for an address",
    "tun_settings — crates/ui/src/connection.rs:406 — AppSettings with TUN enabled",
    "sample_metadata — crates/tray/src/tray.rs:378 — ConnectionMetadata (Manual, Xray, 42 ms)",
    "inline VlessConfig/Subscription fixtures — crates/ui/src/app.rs:1500,1531 — subscription and manual node builders for pure-fn tests",
    "test_paths — crates/core/src/persistence/mod.rs:459 — TempDir + AppPaths (used by tun_session tests, tun_session.rs:61)"
  ],
  "floor": "make lint && make test TEST_TIMEOUT=10m",
  "risks": [
    "Sibling changes fix-connection-ui-state and persist-backend-diagnostics touch app.rs, connection.rs and manager.rs -> planned against HEAD 7d01aaa; land this first or rebase them; conflict hotspots app.rs ProcessStateConnection handler and connection.rs spawn.",
    "strict_route defaults true (core/models/tun.rs:61-74) -> every xray TUN user gets the kill-switch and the IPv6 block on upgrade; mitigated by the CHANGELOG BREAKING entry and the prefs note.",
    "Forwarder races the task's own terminal emit -> abort and join the forwarder before every terminal emit and failover.",
    "Release vs a fresh Connect both take the lifecycle lock, order not guaranteed -> tun_release_in_flight gates Connect until TunReleased.",
    "Error racing a user Disconnect used to schedule an auto-reconnect -> Error while app state Stopping releases and never retries.",
    "Reconnect under a held kill-switch: pin_node_addresses lookups to off-link resolvers fail fast -> hosts not pinned, capture_dns off for that session; on-link resolvers still work via pref 9001; follow-up.",
    "Connection tests spend about 6s per crash-looping candidate on real restart delays -> accepted instead of a public test-only builder; bounded under 30s per test.",
    "A test TunRuntime whose helper resolves to the installed netctl could flush the dev host's table 2023 -> every helper-reaching test uses a tempdir stub; listed as forbidden."
  ],
  "blockersResolved": [
    "process-lifecycle delta: signal-exit sentence reworded to match signal_death_is_treated_as_crash (code is intended)"
  ],
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
