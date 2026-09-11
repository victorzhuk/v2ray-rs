# Design: the running session is the source of truth

## Context

- `AppMsg::ApplyAndRestart` (`crates/ui/src/app.rs:1330`) sets `reconnect_pending` and disconnects; `Stopped` then sends `Connect` (`app.rs:1165`), which builds candidates with `ConnectionPlanner` (`app.rs:1013`). `ConnectToNode` sets `direct_connect_in_flight`, cleared on `Running` (`app.rs:1172`); automatic reconnect (`app.rs:1188`) also sends `Connect`.
- `RuntimeConfigSnapshot::diverges_from` (`crates/core/src/runtime_snapshot.rs:25`) compares backend, binary, ports, listen address, DNS, routing, nodes, subscriptions, strategy and real-delay flag — not `tun` nor the idle/heartbeat settings the generators read.
- The Preferences dialog holds `Rc<RefCell<AppSettings>>` cloned at open (`crates/ui/src/preferences/mod.rs:40`) and emits the whole struct; `FlushSettings` persists it verbatim (`app.rs:940`), while `last_success` is written by the app on `Running` (`app.rs:1119`).
- `schedule_auto_reconnect` (`app.rs:241`) arms a timer tagged with `reconnect_generation`; only `Running` and direct-connect paths cancel it.
- `ProcessLogLine` carries no generation (`crates/ui/src/connection.rs:229`).

## Goals / Non-Goals

**Goals:**
- Every reconnect the app starts on the user's behalf targets what the user chose.
- The restart banner appears whenever the generated config would differ.

**Non-Goals:**
- Changing planned (non-direct) sessions: an explicit apply still replans with the configured strategy, as `connection-auto-resolve` already requires.
- Redesigning the Preferences dialog's state model.

## Decisions

- **Remember the direct target for the session.** Replace the transient `direct_connect_in_flight` with `session_target: Option<ConnectionTarget>` set by `ConnectToNode` and kept until the session ends by user action. `ApplyAndRestart` and automatic reconnect route through `ConnectToNode(target)` when it is set, otherwise `Connect`. A missing or disabled target toasts and falls back to `Connect`. Alternative rejected: putting the running node first in a replanned list — it would fail over to other nodes, which a direct connect explicitly must not.
- **Extend the snapshot, not a byte diff of configs.** Add `tun: TunConfig` and the two timeout values to `RuntimeConfigSnapshot`, compared and restored like the others. Comparing generated JSON would flag every host-pin refresh as a pending change. The TUN-active flag handed to the subscriptions page and tray reads the launched snapshot while connected and settings while disconnected.
- **The app owns `last_success`.** `FlushSettings` overwrites the incoming struct's `last_success` with the app's current value before persisting. The dialog never edits it, so nothing is lost. Alternative rejected: field-level diffs from the dialog — a larger change to every row handler.
- **User actions cancel the reconnect timer.** `Connect` and `Disconnect` call `cancel_auto_reconnect()` unless the message came from `AutoReconnect`, which is marked with a flag on the message.
- **Generation on log lines.** `ProcessLogLine(generation, line)`; the handler drops mismatches, as it does for state messages.

## Risks / Trade-offs

- [A direct session whose node keeps crashing reconnects to the same node up to the automatic budget] → consistent with "direct connect never falls back"; the final give-up surfaces the error.
- [Snapshot schema grows] → in-memory only; nothing persisted.

## Migration Plan

UI-only change, no persisted format change. Rollback is a revert.

## Implementation plan

Base `a015702`, tier standard, mode existing-service-strict, estimate 3 h. Lenses: spec, quality — spec always; quality for a standard tier.

This plan is written against the code after `harden-connection-lifecycle` landed; the Context section above cites the older layout, and the sites below are the current anchors.

Rules for whoever executes it: every path is relative to the worktree; `git -C <worktree>` for every git call; one conventional commit per finished task (subject ≤72 chars, imperative, lowercase); never stage `openspec/`; a test once written is changed only by a new task; test runs stay bounded (`make test-*` carries `timeout 5m` and 4 threads).

Rust has no separate test-writing stage: in every chunk the tests are the first code tasks, the package's other test files stay unchanged, and the chunk closes when its verify command passes.

### snapshot-fields — tasks 1.1

- Shape: first in the serial chain; coder: rust-coder; seam: snapshot-fields.
- Files: `crates/core/src/runtime_snapshot.rs`, `crates/ui/src/app.rs`, `crates/core/src/config/v2ray.rs`, `crates/core/src/config/xray.rs`.
- Tests: `runtime_snapshot::tests::test_runtime_config_snapshot_detects_tun_divergence`, `runtime_snapshot::tests::test_runtime_config_snapshot_detects_idle_timeout_divergence`, `runtime_snapshot::tests::test_runtime_config_snapshot_detects_ws_heartbeat_divergence`, `runtime_snapshot::tests::test_runtime_config_snapshot_restores_tun_and_timeouts`.
- Verify: `make test-core && make test-ui && cargo clippy -p v2ray-rs-core -p v2ray-rs-ui --all-targets --all-features -- -D warnings`
- Steps:
  - Tests in crates/core/src/runtime_snapshot.rs: make_snapshot and every RuntimeConfigSnapshot literal (lines 79, 147, 220, 240, 321) get tun: TunConfig::default(), idle_timeout_secs: AppSettings::default().idle_timeout_secs, ws_heartbeat_secs: 0 so existing no-divergence asserts keep holding
  - Add test_runtime_config_snapshot_detects_tun_divergence (settings.tun.enabled, then interface_name, then strict_route each flips diverges_from to true from a matching baseline), test_runtime_config_snapshot_detects_idle_timeout_divergence, test_runtime_config_snapshot_detects_ws_heartbeat_divergence, test_runtime_config_snapshot_restores_tun_and_timeouts
  - Code: add pub tun: TunConfig, pub idle_timeout_secs: u32, pub ws_heartbeat_secs: u32 to RuntimeConfigSnapshot; diverges_from adds self.tun != settings.tun || self.idle_timeout_secs != settings.idle_timeout_secs || self.ws_heartbeat_secs != settings.ws_heartbeat_secs; restore_settings writes all three
  - Code: crates/ui/src/app.rs:489-502 capture tun: self.settings.tun.clone(), idle_timeout_secs, ws_heartbeat_secs from self.settings
  - AMENDMENT: runtime_snapshot.rs test literal at `..make_snapshot` is a struct-update literal and needs no edit; only the full RuntimeConfigSnapshot literals need the new fields.

### tun-from-snapshot — tasks 1.2

- Shape: after `snapshot-fields` (shares crates/ui); coder: rust-coder; seam: tun-from-snapshot.
- Files: `crates/ui/src/app.rs`, `crates/ui/src/subscriptions.rs`, `crates/ui/src/connection.rs`.
- Tests: `app::tests::tun_active_follows_launched_snapshot`, `app::tests::tun_active_false_outside_running`, `connection::tests::no_marker_when_launched_without_tun`, `connection::tests::marker_written_from_runtime_before_start`.
- Verify: `make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings`
- Steps:
  - Test app::tests::tun_active_follows_launched_snapshot: snapshot with tun disabled -> false; tun enabled + Xray -> true; tun enabled + V2ray -> false (all with ProcessState::Running)
  - Test app::tests::tun_active_false_outside_running: Starting/Stopping/Stopped/Error with a TUN snapshot -> false; Running with None -> false
  - Test connection::tests::no_marker_when_launched_without_tun: stub 'exit 1', singbox_settings() (tun disabled), one candidate; wait for Error; assert load_tun_session(&stub.paths) is None
  - Code: fn tun_active_for(state: &ProcessState, snapshot: Option<&RuntimeConfigSnapshot>) -> bool = Running && snapshot.is_some_and(|s| s.tun.enabled && s.backend_type != BackendType::V2ray); apply_state uses it with self.runtime_snapshot.as_ref()

### reconnect-yields — tasks 3.2

- Shape: after `tun-from-snapshot` (shares crates/ui); coder: rust-coder; seam: reconnect-yields.
- Files: `crates/ui/src/app.rs`.
- Tests: `app::tests::user_connect_cancels_pending_auto_reconnect`, `app::tests::auto_reconnect_connect_keeps_budget`, `app::tests::auto_reconnect_fires_only_for_current_generation_without_handle`, `app::tests::auto_reconnect_exhausted_after_three_attempts`.
- Verify: `make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings`
- Steps:
  - Tests: cancels_auto_reconnect(User) true, (AutoReconnect) false; auto_reconnect_fires(g, g.wrapping_add(1), false) false, (g, g, false) true, (g, g, true) false
  - Code: #[derive(Debug, Clone, Copy, PartialEq, Eq)] enum ConnectOrigin { User, AutoReconnect }; AppMsg::Connect(ConnectOrigin); AppMsg::ConnectToNode(ConnectionNodeRef, ConnectOrigin)
  - Code: fn cancels_auto_reconnect(origin: ConnectOrigin) -> bool (origin != AutoReconnect); fn auto_reconnect_fires(message: u32, current: u32, has_handle: bool) -> bool extracted from app.rs:1237
  - Code: Connect handler calls self.cancel_auto_reconnect() right after the handle/tun_release_in_flight guard (app.rs:1021-1023) when cancels_auto_reconnect(origin); ConnectToNode guards its two existing cancel calls with the same fn
  - Code: construct sites: app.rs:767 tray and 1017 toggle -> Connect(User); 812/826 page forwards and 1200 pending replay -> ConnectToNode(_, User); 1208 -> Connect(User) (restart-via-target replaces it); 1238 -> Connect(AutoReconnect)
  - AMENDMENT (supersedes the cancel placement above): in the Connect handler, call cancel_auto_reconnect() only after the candidates.is_empty() check, immediately before start_connection — a user Connect that fails to load stores, finds no candidates, or never reaches start_connection leaves the timer armed, the same rule as an unresolvable ConnectToNode.
  - AMENDMENT: test binding — user_connect_cancels_pending_auto_reconnect asserts cancels_auto_reconnect(ConnectOrigin::User) is true; auto_reconnect_connect_keeps_budget asserts cancels_auto_reconnect(ConnectOrigin::AutoReconnect) is false; auto_reconnect_fires_only_for_current_generation_without_handle asserts auto_reconnect_fires(g, g.wrapping_add(1), false) is false, (g, g, false) is true, (g, g, true) is false. auto_reconnect_exhausted_after_three_attempts already exists and must stay green.

### session-target — tasks 2.1, 2.3

- Shape: after `reconnect-yields` (shares crates/ui); coder: rust-coder; seam: session-target.
- Files: `crates/ui/src/app.rs`.
- Tests: `app::tests::direct_session_starts_unestablished`, `app::tests::session_target_kept_across_running_and_established`, `app::tests::session_target_kept_on_stop_during_restart`, `app::tests::session_target_cleared_on_stop_without_restart`, `app::tests::failed_direct_connect_gets_no_retry`, `app::tests::established_direct_session_retries_within_budget`, `app::tests::planned_error_retry_unchanged`, `app::tests::error_replays_pending_direct_target_before_any_other_reconnect`, `app::tests::stopped_replays_pending_direct_target_before_any_other_reconnect`, `app::tests::error_without_pending_target_replays_nothing`, `app::tests::stopped_without_pending_target_replays_nothing`, `app::tests::error_while_stopping_releases_without_retry`, `app::tests::error_with_reconnects_left_keeps_killswitch`.
- Verify: `make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings`
- Steps:
  - Tests (names in targetedTests): direct_session User -> established false, AutoReconnect -> true; after_running sets established; after_stop(t,true)=t, (t,false)=None (covers user Disconnect); retry_after_error: unestablished -> false, established or None -> !release_on_error(stopping, left)
  - Tests 2.3: keep error_/stopped_replays_pending_direct_target_before_any_other_reconnect names and replay/pending asserts, drop in-flight arg; the two in-flight-only tests become error_/stopped_without_pending_target_replays_nothing (blockers)
  - Code: #[derive(Debug, Clone, Copy, PartialEq, Eq)] struct SessionTarget { node: ConnectionNodeRef, established: bool }; App field session_target replaces direct_connect_in_flight (app.rs:86, 893)
  - Code: fn direct_session(node, origin) -> SessionTarget; fn session_target_after_running(Option<SessionTarget>) -> Option<SessionTarget>; fn session_target_after_stop(Option<SessionTarget>, reconnect_pending: bool) -> Option<SessionTarget>; fn retry_after_error(target: Option<SessionTarget>, app_state_stopping: bool, reconnects_left: u32) -> bool; consume_terminal_direct_state(state, &mut Option<ConnectionNodeRef>) -> Option<ConnectionNodeRef>
  - Code wiring: ConnectToNode start Ok -> Some(direct_session(target, origin)), Err -> None (app.rs:1118-1123); Connect sets None when it calls start_connection; Running arm -> after_running (1213); Stopped arm -> after_stop(.., self.reconnect_pending) (1229-1231); Disconnect evaluates after_stop with reconnect_pending read at handler entry (1125); Error arm (1215-1228): let retry = retry_after_error(self.session_target, was_stopping, left) && self.schedule_auto_reconnect(&sender); if !retry { self.session_target = None; release if marker present }
  - AMENDMENT: the codemap site notes that say 'replace the bool with Option<ConnectionNodeRef>' and 'the Disconnect ConnectToNode sends while running must not clear the target' describe HEAD only — the codeTasks govern: the field is Option<SessionTarget { node, established }>, and that internal Disconnect does clear it (the pending replay sets a fresh target).

### restart-via-target — tasks 2.2

- Shape: after `session-target` (shares crates/ui); coder: rust-coder; seam: restart-via-target.
- Files: `crates/ui/src/app.rs`.
- Tests: `app::tests::restart_reconnects_direct_session_to_its_node`, `app::tests::restart_without_direct_session_replans`, `app::tests::auto_reconnect_keeps_direct_node`, `app::tests::unavailable_target_falls_back_only_for_system_reconnects`, `app::tests::restart_origin_cancels_like_user`.
- Verify: `make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings`
- Steps:
  - Tests: matches! on reconnect_msg(Some(t), Restart|AutoReconnect) -> ConnectToNode(t.node, origin), None -> Connect(origin); falls_back_to_planner User false, Restart/AutoReconnect true; cancels_auto_reconnect(Restart) true
  - Code: ConnectOrigin::Restart; fn reconnect_msg(target: Option<SessionTarget>, origin: ConnectOrigin) -> AppMsg; fn falls_back_to_planner(origin: ConnectOrigin) -> bool
  - Code: app.rs:1206-1208 sends reconnect_msg(self.session_target, ConnectOrigin::Restart); app.rs:1237-1238 sends reconnect_msg(self.session_target, ConnectOrigin::AutoReconnect)
  - Code: ConnectToNode resolve failure (app.rs:1100-1105): if falls_back_to_planner(origin) { toast 'Chosen node is unavailable, reconnecting with the configured strategy'; self.session_target = None; sender.input(AppMsg::Connect(origin)) } else HEAD toast
  - AMENDMENT: restart_origin_cancels_like_user also asserts direct_session(node, ConnectOrigin::Restart).established == false, so a restart attempt that fails is not retried and releases the kill-switch.

### last-success-owned — tasks 3.1

- Shape: after `restart-via-target` (shares crates/ui); coder: rust-coder; seam: last-success-owned.
- Files: `crates/ui/src/app.rs`, `crates/ui/src/preferences/mod.rs`.
- Tests: `app::tests::flush_keeps_current_last_success`, `app::tests::flush_does_not_invent_last_success`, `app::tests::flush_keeps_other_fields`, `app::tests::direct_session_success_records_last_success`.
- Verify: `make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings`
- Steps:
  - Tests: app::tests::flush_keeps_current_last_success (incoming last_success = stale A, current = B -> result B); flush_does_not_invent_last_success (incoming Some(A), current None -> None); flush_keeps_other_fields (incoming socks_port changed survives)
  - Code: fn keep_last_success(incoming: AppSettings, current: &AppSettings) -> AppSettings; FlushSettings persists keep_last_success(settings, &self.settings)
  - AMENDMENT (write this test first, with the other tests of this chunk): app::tests::direct_session_success_records_last_success — the last-success value written on Running for a session started by ConnectToNode names that node (extract the Running-arm last-success builder as a pure fn if it is not one already).

### log-generation — tasks 4.1

- Shape: after `last-success-owned` (shares crates/ui); coder: rust-coder; seam: log-generation.
- Files: `crates/ui/src/app.rs`, `crates/ui/src/connection.rs`.
- Tests: `connection::tests::log_lines_carry_connection_generation`, `app::tests::interleaved_generations_keep_only_current`.
- Verify: `make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings`
- Steps:
  - Test connection::tests::log_lines_carry_connection_generation: stub '[ "$1" = check ] && exit 0; while :; do echo v2rs-log-line; sleep 0.2; done' with singbox_settings(); receive until AppMsg::ProcessLogLine(g, line) with line == v2rs-log-line; assert g == GENERATION; handle.stop()
  - Test app::tests::interleaved_generations_keep_only_current: [(6, old), (7, new), (6, late)] filtered by is_current_generation(g, 7) -> [new]
  - Code: AppMsg::ProcessLogLine(u64, String); connection.rs:249 emits ProcessLogLine(generation, line.content); fn is_current_generation(message: u64, current: u64) -> bool used at app.rs:1150 and in the ProcessLogLine arm before LogsMsg::AppendLine
  - AMENDMENT: CHANGELOG.md [Unreleased] Fixed (Keep a Changelog, match existing entry style), one line each: Apply & Restart and automatic reconnect keep a directly chosen node; editing TUN, idle-timeout or heartbeat settings while connected raises the restart banner; saving Preferences no longer rolls back the last successful node; a manual Connect cancels a pending automatic reconnect; log lines from a replaced connection no longer appear.

### verification — tasks 5.1, 5.2

- Shape: after `log-generation` (shares workspace); coder: zpatcher; seam: verification.
- Files: `Makefile`, `crates/ui/src/app.rs`.
- Verify: `make lint && make test TEST_TIMEOUT=10m`

### Waivers

- snapshot-fields: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- tun-from-snapshot: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- reconnect-yields: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- session-target: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- restart-via-target: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- last-success-owned: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- log-generation: NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command.
- verification: NO-RED-WAIVER: verification; NO-TESTER-WAIVER: 5.

### Floor

`make lint && make test TEST_TIMEOUT=10m (matches ci.yml)`

Task 5.2 is a live check on a running app, done by hand after merge.

### Risks

- persist-backend-diagnostics overlaps app.rs (failure toasts) and connection.rs (3.3 passes the backend log path through ConnectionRequest), plus manager.rs reader tasks -> land this change first; its ConnectionRequest field and log-file writes are independent of the ProcessLogLine tag, rebase is textual
- AppMsg::Connect/ConnectToNode/ProcessLogLine shape change touches every construct site -> compiler-enforced; list in reconnect-yields codeTasks
- Direct node that keeps crashing retries 3 x 5s before give-up -> accepted in design risks; final Error releases the kill-switch
- Chunk interim: between session-target and restart-via-target an established direct session auto-reconnects via the planner -> chunks land together, not merged separately
- CHANGELOG [Unreleased] entry not in tasks.md -> add a Fixed entry with the verification chunk

### Plan review

Pass on the first round by an independent reviewer, with no blockers. Its warnings became the AMENDMENT steps above: the Connect handler cancels the reconnect timer only just before start_connection, the test-to-assertion bindings in reconnect-yields, a Restart-origin direct session starts unestablished, site notes that describe HEAD only, the last-success test written first, and the CHANGELOG entry moved into log-generation. The four interactions with the kill-switch change were checked against the current code and preserved. Accepted gap: clearing the session target on an ordinary Connect, and on a failed direct start, is handler wiring with no test; the live check in 5.2 covers it. A gap that already existed stays out of scope: if an Error replays a pending direct target that no longer resolves, the kill-switch routes stay installed with no process.

## Plan appendix

```json
{
  "v": 2,
  "change": "fix-connection-ui-state",
  "baseSha": "a0157022f11d30636ec114722790f667e3d56e44",
  "generatedAt": "2026-09-11T18:45:17.623Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "estimateHours": 3,
  "chunks": [
    {
      "id": "snapshot-fields",
      "taskIds": [
        "1.1"
      ],
      "seam": "snapshot-fields",
      "contract": {
        "states": [
          "snapshot.tun",
          "snapshot.idle_timeout_secs",
          "snapshot.ws_heartbeat_secs"
        ],
        "transitions": [
          {
            "input": "any TunConfig field in settings differs from snapshot",
            "state": "diverges_from",
            "effect": "set",
            "evidence": "process-lifecycle spec: TUN edit raises the restart banner"
          },
          {
            "input": "settings.idle_timeout_secs differs",
            "state": "diverges_from",
            "effect": "set",
            "evidence": "config/v2ray.rs:59 connIdle reads it"
          },
          {
            "input": "settings.ws_heartbeat_secs differs",
            "state": "diverges_from",
            "effect": "set",
            "evidence": "config/xray.rs:110 apply_ws_heartbeat"
          },
          {
            "input": "all three equal",
            "state": "diverges_from",
            "effect": "no-op",
            "evidence": "runtime_snapshot.rs:136"
          },
          {
            "input": "restore_settings",
            "state": "settings.tun/idle/heartbeat",
            "effect": "forced",
            "evidence": "runtime_snapshot.rs:45-54"
          },
          {
            "input": "start_connection",
            "state": "snapshot fields",
            "effect": "set",
            "evidence": "app.rs:489-502"
          }
        ],
        "forbidden": [
          "a fixture baseline whose tun/idle/heartbeat differ from AppSettings::default() while the test asserts no divergence"
        ],
        "seeding": [
          "RuntimeConfigSnapshot literal via make_snapshot plus struct update (..make_snapshot(..)); settings mutated field by field from AppSettings default"
        ],
        "budgets": [
          "make test-core wall clock <= 5m (Makefile TEST_TIMEOUT)"
        ],
        "names": [
          "RuntimeConfigSnapshot::tun",
          "RuntimeConfigSnapshot::idle_timeout_secs",
          "RuntimeConfigSnapshot::ws_heartbeat_secs",
          "AppSettings::tun",
          "AppSettings::idle_timeout_secs",
          "AppSettings::ws_heartbeat_secs",
          "TunConfig::interface_name",
          "TunConfig::strict_route",
          "TunConfig::enabled"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "Tests in crates/core/src/runtime_snapshot.rs: make_snapshot and every RuntimeConfigSnapshot literal (lines 79, 147, 220, 240, 321) get tun: TunConfig::default(), idle_timeout_secs: AppSettings::default().idle_timeout_secs, ws_heartbeat_secs: 0 so existing no-divergence asserts keep holding",
        "Add test_runtime_config_snapshot_detects_tun_divergence (settings.tun.enabled, then interface_name, then strict_route each flips diverges_from to true from a matching baseline), test_runtime_config_snapshot_detects_idle_timeout_divergence, test_runtime_config_snapshot_detects_ws_heartbeat_divergence, test_runtime_config_snapshot_restores_tun_and_timeouts",
        "Code: add pub tun: TunConfig, pub idle_timeout_secs: u32, pub ws_heartbeat_secs: u32 to RuntimeConfigSnapshot; diverges_from adds self.tun != settings.tun || self.idle_timeout_secs != settings.idle_timeout_secs || self.ws_heartbeat_secs != settings.ws_heartbeat_secs; restore_settings writes all three",
        "Code: crates/ui/src/app.rs:489-502 capture tun: self.settings.tun.clone(), idle_timeout_secs, ws_heartbeat_secs from self.settings",
        "AMENDMENT: runtime_snapshot.rs test literal at `..make_snapshot` is a struct-update literal and needs no edit; only the full RuntimeConfigSnapshot literals need the new fields."
      ],
      "coder": "rust-coder",
      "verify": "make test-core && make test-ui && cargo clippy -p v2ray-rs-core -p v2ray-rs-ui --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/runtime_snapshot.rs",
          "symbol": "RuntimeConfigSnapshot",
          "anchor": "pub struct RuntimeConfigSnapshot {",
          "lines": "8-22",
          "change": "add fields tun: TunConfig, idle_timeout_secs: u32, ws_heartbeat_secs: u32; import TunConfig into the use crate::models list (TunConfig derives Debug, Clone, PartialEq at crates/core/src/models/tun.rs:29)"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/runtime_snapshot.rs",
          "symbol": "RuntimeConfigSnapshot::diverges_from",
          "anchor": "|| self.use_real_delay_for_lowest_latency != settings.real_delay.use_for_lowest_latency",
          "lines": "25-43",
          "change": "add || self.tun != settings.tun, || self.idle_timeout_secs != settings.idle_timeout_secs, || self.ws_heartbeat_secs != settings.ws_heartbeat_secs"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/runtime_snapshot.rs",
          "symbol": "RuntimeConfigSnapshot::restore_settings",
          "anchor": "settings.real_delay.use_for_lowest_latency = self.use_real_delay_for_lowest_latency;",
          "lines": "45-54",
          "change": "restore settings.tun (clone), settings.idle_timeout_secs, settings.ws_heartbeat_secs"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/runtime_snapshot.rs",
          "symbol": "tests::make_snapshot + full struct literals",
          "anchor": "fn make_snapshot(backend_type: BackendType, binary_path: &str) -> RuntimeConfigSnapshot {",
          "lines": "78-93; literals also at 147-160, 240-266, 321-334",
          "change": "add the three new fields to make_snapshot and to the three exhaustive struct literals (they will not compile otherwise); NEW tests: each new field raises divergence, restore_settings copies them"
        },
        {
          "task": "1.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::start_connection (snapshot capture)",
          "anchor": "use_real_delay_for_lowest_latency: self.settings.real_delay.use_for_lowest_latency,",
          "lines": "489-502",
          "change": "capture tun: self.settings.tun.clone(), idle_timeout_secs, ws_heartbeat_secs from self.settings (only production construction site of the struct)"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "idle-timeout read (generator)",
          "anchor": "settings.idle_timeout_secs } }",
          "lines": "59",
          "change": "reference only: v2ray-family policy connIdle reads AppSettings.idle_timeout_secs (field: crates/core/src/models/settings.rs:146)"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/config/xray.rs",
          "symbol": "apply_ws_heartbeat call (generator)",
          "anchor": "apply_ws_heartbeat(outbound, settings.ws_heartbeat_secs);",
          "lines": "110-125",
          "change": "reference only: xray wsSettings.heartbeatPeriod reads AppSettings.ws_heartbeat_secs (field: crates/core/src/models/settings.rs:151); no sing-box reader of either field"
        }
      ],
      "pkgDirs": [
        "crates/core",
        "crates/ui"
      ],
      "pkgs": [
        "v2ray-rs-core",
        "v2ray-rs-ui"
      ],
      "parallel": false,
      "shard": "",
      "prev": null,
      "sharedPkg": "crates/ui",
      "targetedTests": [
        "runtime_snapshot::tests::test_runtime_config_snapshot_detects_tun_divergence",
        "runtime_snapshot::tests::test_runtime_config_snapshot_detects_idle_timeout_divergence",
        "runtime_snapshot::tests::test_runtime_config_snapshot_detects_ws_heartbeat_divergence",
        "runtime_snapshot::tests::test_runtime_config_snapshot_restores_tun_and_timeouts"
      ]
    },
    {
      "id": "tun-from-snapshot",
      "taskIds": [
        "1.2"
      ],
      "seam": "tun-from-snapshot",
      "contract": {
        "states": [
          "tun_active"
        ],
        "transitions": [
          {
            "input": "Running, snapshot.tun.enabled, backend Xray|SingBox",
            "state": "tun_active",
            "effect": "set",
            "evidence": "app.rs:153-157"
          },
          {
            "input": "Running, snapshot.tun disabled, settings.tun enabled mid-session",
            "state": "tun_active",
            "effect": "clear",
            "evidence": "spec scenario: TUN indicators follow the running session"
          },
          {
            "input": "Running, snapshot backend V2ray",
            "state": "tun_active",
            "effect": "clear",
            "evidence": "app.rs:155"
          },
          {
            "input": "any non-Running state",
            "state": "tun_active",
            "effect": "clear",
            "evidence": "app.rs:153"
          },
          {
            "input": "Running with runtime_snapshot None",
            "state": "tun_active",
            "effect": "clear",
            "evidence": "clear_restart_flow app.rs:210-213 may run before a late relay"
          },
          {
            "input": "settings.tun edited mid-session",
            "state": "tun_session.json",
            "effect": "no-op",
            "evidence": "connection.rs:172-179 reads launched settings"
          }
        ],
        "forbidden": [
          "tun_active reading self.settings while Running"
        ],
        "seeding": [
          "pure fn inputs: RuntimeConfigSnapshot literal in app tests; marker via the real connection task with a stub backend (connection.rs:443-493 helpers)"
        ],
        "budgets": [
          "connection test RECV_TIMEOUT 20s (connection.rs:435); make test-ui <= 5m"
        ],
        "names": [
          "tun_active_for",
          "SubscriptionsMsg::SetTunActive",
          "load_tun_session",
          "singbox_settings"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "Test app::tests::tun_active_follows_launched_snapshot: snapshot with tun disabled -> false; tun enabled + Xray -> true; tun enabled + V2ray -> false (all with ProcessState::Running)",
        "Test app::tests::tun_active_false_outside_running: Starting/Stopping/Stopped/Error with a TUN snapshot -> false; Running with None -> false",
        "Test connection::tests::no_marker_when_launched_without_tun: stub 'exit 1', singbox_settings() (tun disabled), one candidate; wait for Error; assert load_tun_session(&stub.paths) is None",
        "Code: fn tun_active_for(state: &ProcessState, snapshot: Option<&RuntimeConfigSnapshot>) -> bool = Running && snapshot.is_some_and(|s| s.tun.enabled && s.backend_type != BackendType::V2ray); apply_state uses it with self.runtime_snapshot.as_ref()"
      ],
      "coder": "rust-coder",
      "verify": "make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "1.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::apply_state (tun_active)",
          "anchor": "&& self.settings.tun.enabled",
          "lines": "153-157",
          "change": "derive tun_active from self.runtime_snapshot (tun.enabled + backend_type != V2ray) while connected, settings while disconnected; snapshot is set before apply_state(Starting) at 508 and cleared by clear_restart_flow on terminal states; candidate NEW pure fn for unit test"
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/subscriptions.rs",
          "symbol": "SubscriptionsMsg::SetTunActive handler / probe gate",
          "anchor": "SubscriptionsMsg::SetTunActive(active) => {",
          "lines": "723-727; gate at 524, subscriptions_eligible_for_latency_test at 1187",
          "change": "reference only: consumer of the flag emitted by apply_state; no edit expected"
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "tray feed in App::apply_state",
          "anchor": "if let Ok(guard) = TRAY_EVENT_TX.lock()",
          "lines": "159-167",
          "change": "reference only: tray receives ProcessEvent::StateChanged {from,to,connection}; crates/tray/src has no TUN reader (see MISSING in patterns)"
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn: TUN marker write",
          "anchor": "&& let Err(err) = save_tun_session(&paths, &tun_session_for(rt))",
          "lines": "172-179",
          "change": "reference only: marker built from build_tun_runtime(&effective_settings) where settings is the by-value ConnectionRequest.settings cloned at app.rs:512 in the same start_connection call that captures the snapshot; a mid-session TUN edit cannot write a marker for the running connection; no app.rs marker writer remains (app.rs only clears at 1163 and reads the file via tun_marker_present at 237)"
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
      "prev": "snapshot-fields",
      "sharedPkg": "crates/ui",
      "targetedTests": [
        "app::tests::tun_active_follows_launched_snapshot",
        "app::tests::tun_active_false_outside_running",
        "connection::tests::no_marker_when_launched_without_tun",
        "connection::tests::marker_written_from_runtime_before_start"
      ]
    },
    {
      "id": "reconnect-yields",
      "taskIds": [
        "3.2"
      ],
      "seam": "reconnect-yields",
      "contract": {
        "states": [
          "reconnect_generation",
          "auto_reconnect_attempts"
        ],
        "transitions": [
          {
            "input": "Connect(User), no handle, no release in flight",
            "state": "reconnect_generation/attempts",
            "effect": "clear",
            "evidence": "spec: Manual connect during the reconnect delay"
          },
          {
            "input": "Connect(AutoReconnect)",
            "state": "reconnect_generation/attempts",
            "effect": "no-op",
            "evidence": "design: unless the message came from AutoReconnect"
          },
          {
            "input": "Connect(*) with handle or tun_release_in_flight",
            "state": "reconnect_generation/attempts",
            "effect": "no-op",
            "evidence": "app.rs:1021-1023"
          },
          {
            "input": "ConnectToNode(t, User) resolvable",
            "state": "reconnect_generation/attempts",
            "effect": "clear",
            "evidence": "app.rs:1108,1115"
          },
          {
            "input": "ConnectToNode(t, User) unresolvable",
            "state": "reconnect_generation/attempts",
            "effect": "no-op",
            "evidence": "app.rs:1074-1076"
          },
          {
            "input": "ConnectToNode(t, AutoReconnect)",
            "state": "reconnect_generation/attempts",
            "effect": "no-op",
            "evidence": "budget must count toward MAX_AUTO_RECONNECTS"
          },
          {
            "input": "Disconnect",
            "state": "reconnect_generation/attempts",
            "effect": "clear",
            "evidence": "app.rs:1127 (satisfied)"
          },
          {
            "input": "AutoReconnect(g), g == current, no handle",
            "state": "fires",
            "effect": "set",
            "evidence": "app.rs:1237"
          },
          {
            "input": "AutoReconnect(g), g != current",
            "state": "fires",
            "effect": "no-op",
            "evidence": "app.rs:1237"
          }
        ],
        "forbidden": [
          "an AutoReconnect-originated connect resetting auto_reconnect_attempts",
          "a user Connect followed by the old timer firing Connect"
        ],
        "seeding": [
          "generations are plain u32 inputs to auto_reconnect_fires; cancellation modeled as wrapping_add(1) exactly as cancel_auto_reconnect (app.rs:217-220)"
        ],
        "budgets": [
          "AUTO_RECONNECT_DELAY = 5s (app.rs:36)",
          "MAX_AUTO_RECONNECTS = 3 (app.rs:35)",
          "make test-ui <= 5m"
        ],
        "names": [
          "ConnectOrigin",
          "ConnectOrigin::User",
          "ConnectOrigin::AutoReconnect",
          "AppMsg::Connect(ConnectOrigin)",
          "AppMsg::ConnectToNode(ConnectionNodeRef, ConnectOrigin)",
          "cancels_auto_reconnect",
          "auto_reconnect_fires",
          "cancel_auto_reconnect",
          "reconnect_generation"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "Tests: cancels_auto_reconnect(User) true, (AutoReconnect) false; auto_reconnect_fires(g, g.wrapping_add(1), false) false, (g, g, false) true, (g, g, true) false",
        "Code: #[derive(Debug, Clone, Copy, PartialEq, Eq)] enum ConnectOrigin { User, AutoReconnect }; AppMsg::Connect(ConnectOrigin); AppMsg::ConnectToNode(ConnectionNodeRef, ConnectOrigin)",
        "Code: fn cancels_auto_reconnect(origin: ConnectOrigin) -> bool (origin != AutoReconnect); fn auto_reconnect_fires(message: u32, current: u32, has_handle: bool) -> bool extracted from app.rs:1237",
        "Code: Connect handler calls self.cancel_auto_reconnect() right after the handle/tun_release_in_flight guard (app.rs:1021-1023) when cancels_auto_reconnect(origin); ConnectToNode guards its two existing cancel calls with the same fn",
        "Code: construct sites: app.rs:767 tray and 1017 toggle -> Connect(User); 812/826 page forwards and 1200 pending replay -> ConnectToNode(_, User); 1208 -> Connect(User) (restart-via-target replaces it); 1238 -> Connect(AutoReconnect)",
        "AMENDMENT (supersedes the cancel placement above): in the Connect handler, call cancel_auto_reconnect() only after the candidates.is_empty() check, immediately before start_connection — a user Connect that fails to load stores, finds no candidates, or never reaches start_connection leaves the timer armed, the same rule as an unresolvable ConnectToNode.",
        "AMENDMENT: test binding — user_connect_cancels_pending_auto_reconnect asserts cancels_auto_reconnect(ConnectOrigin::User) is true; auto_reconnect_connect_keeps_budget asserts cancels_auto_reconnect(ConnectOrigin::AutoReconnect) is false; auto_reconnect_fires_only_for_current_generation_without_handle asserts auto_reconnect_fires(g, g.wrapping_add(1), false) is false, (g, g, false) is true, (g, g, true) is false. auto_reconnect_exhausted_after_three_attempts already exists and must stay green."
      ],
      "coder": "rust-coder",
      "verify": "make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "3.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::Connect handler",
          "anchor": "if self.process_handle.is_some() || self.tun_release_in_flight {",
          "lines": "1020-1069",
          "change": "no cancel_auto_reconnect at HEAD; add it for user-originated Connect only (AutoReconnect path must keep the attempt budget)"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg enum (Connect / AutoReconnect(u32))",
          "anchor": "pub enum AppMsg {",
          "lines": "94-121",
          "change": "mark AutoReconnect-originated connects (flag on Connect or ConnectToNode) per design; Connect currently a unit variant sent from ToggleConnection 1017, reconnect_after_stop 1208, AutoReconnect 1238"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::cancel_auto_reconnect",
          "anchor": "fn cancel_auto_reconnect(&mut self) {",
          "lines": "215-220",
          "change": "reference: bumps reconnect_generation and resets auto_reconnect_attempts; already called by Disconnect 1127, ConnectToNode 1108/1115, Running 1212, replay 1199, quit 263"
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
      "prev": "tun-from-snapshot",
      "sharedPkg": "crates/ui",
      "targetedTests": [
        "app::tests::user_connect_cancels_pending_auto_reconnect",
        "app::tests::auto_reconnect_connect_keeps_budget",
        "app::tests::auto_reconnect_fires_only_for_current_generation_without_handle",
        "app::tests::auto_reconnect_exhausted_after_three_attempts"
      ]
    },
    {
      "id": "session-target",
      "taskIds": [
        "2.1",
        "2.3"
      ],
      "seam": "session-target",
      "contract": {
        "states": [
          "session_target",
          "session_target.established",
          "tun_session.json (kill-switch)"
        ],
        "transitions": [
          {
            "input": "ConnectToNode(t, User|Restart) start Ok",
            "state": "session_target",
            "effect": "set",
            "evidence": "design: set by ConnectToNode; established=false"
          },
          {
            "input": "ConnectToNode(t, AutoReconnect) start Ok",
            "state": "session_target",
            "effect": "set",
            "evidence": "established=true, auto path only follows an established session"
          },
          {
            "input": "ConnectToNode start_connection Err",
            "state": "session_target",
            "effect": "clear",
            "evidence": "app.rs:1118-1123"
          },
          {
            "input": "ConnectToNode(t, User) unresolvable",
            "state": "session_target",
            "effect": "no-op",
            "evidence": "app.rs:1074-1076"
          },
          {
            "input": "Connect(*) reaching start_connection",
            "state": "session_target",
            "effect": "clear",
            "evidence": "planned session replaces direct; spec: ordinary connects use the strategy"
          },
          {
            "input": "Running",
            "state": "session_target.established",
            "effect": "set",
            "evidence": "tasks 2.1 keep across Running"
          },
          {
            "input": "Stopped with reconnect_pending",
            "state": "session_target",
            "effect": "no-op",
            "evidence": "ApplyAndRestart app.rs:1377-1381"
          },
          {
            "input": "Stopped without reconnect_pending",
            "state": "session_target",
            "effect": "clear",
            "evidence": "app.rs:1229-1231"
          },
          {
            "input": "user Disconnect (reconnect_pending false at entry)",
            "state": "session_target",
            "effect": "clear",
            "evidence": "tasks 2.1 clear on Disconnect"
          },
          {
            "input": "Disconnect sent by ApplyAndRestart (reconnect_pending true)",
            "state": "session_target",
            "effect": "no-op",
            "evidence": "app.rs:1379-1380"
          },
          {
            "input": "Error, target unestablished",
            "state": "session_target + kill-switch",
            "effect": "clear",
            "evidence": "app.rs:1216-1218,1225-1227; spec: direct connect failure surfaces immediately"
          },
          {
            "input": "Error, target established, !release_on_error, schedule true",
            "state": "session_target + kill-switch",
            "effect": "no-op",
            "evidence": "spec: automatic reconnect keeps the chosen node; harden release_on_error app.rs:1457"
          },
          {
            "input": "Error, target established, stopping or budget spent or pending_exit",
            "state": "session_target + kill-switch",
            "effect": "clear",
            "evidence": "app.rs:1225-1227"
          },
          {
            "input": "Error, None",
            "state": "kill-switch",
            "effect": "no-op",
            "evidence": "planned path unchanged app.rs:1220-1223"
          },
          {
            "input": "Stopped|Error with pending_direct_target",
            "state": "pending_direct_target",
            "effect": "clear",
            "evidence": "app.rs:1193-1201, replay sets a new session_target"
          }
        ],
        "forbidden": [
          "retry scheduled for an unestablished direct target",
          "tun marker released while a retry is scheduled",
          "ApplyAndRestart's Disconnect clearing session_target",
          "established=true reached except through Running or an AutoReconnect ConnectToNode"
        ],
        "seeding": [
          "pure fn inputs only; Some(SessionTarget{established:false}) via direct_session(node, ConnectOrigin::User); established=true via session_target_after_running(Some(direct_session(..))); node = ConnectionNodeRef::Manual { node_id: uuid::Uuid::nil() }"
        ],
        "budgets": [
          "MAX_AUTO_RECONNECTS = 3 retries for an established direct session",
          "make test-ui <= 5m"
        ],
        "names": [
          "SessionTarget",
          "SessionTarget::node",
          "SessionTarget::established",
          "App::session_target",
          "direct_session",
          "session_target_after_running",
          "session_target_after_stop",
          "retry_after_error",
          "release_on_error",
          "consume_terminal_direct_state",
          "pending_direct_target"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "Tests (names in targetedTests): direct_session User -> established false, AutoReconnect -> true; after_running sets established; after_stop(t,true)=t, (t,false)=None (covers user Disconnect); retry_after_error: unestablished -> false, established or None -> !release_on_error(stopping, left)",
        "Tests 2.3: keep error_/stopped_replays_pending_direct_target_before_any_other_reconnect names and replay/pending asserts, drop in-flight arg; the two in-flight-only tests become error_/stopped_without_pending_target_replays_nothing (blockers)",
        "Code: #[derive(Debug, Clone, Copy, PartialEq, Eq)] struct SessionTarget { node: ConnectionNodeRef, established: bool }; App field session_target replaces direct_connect_in_flight (app.rs:86, 893)",
        "Code: fn direct_session(node, origin) -> SessionTarget; fn session_target_after_running(Option<SessionTarget>) -> Option<SessionTarget>; fn session_target_after_stop(Option<SessionTarget>, reconnect_pending: bool) -> Option<SessionTarget>; fn retry_after_error(target: Option<SessionTarget>, app_state_stopping: bool, reconnects_left: u32) -> bool; consume_terminal_direct_state(state, &mut Option<ConnectionNodeRef>) -> Option<ConnectionNodeRef>",
        "Code wiring: ConnectToNode start Ok -> Some(direct_session(target, origin)), Err -> None (app.rs:1118-1123); Connect sets None when it calls start_connection; Running arm -> after_running (1213); Stopped arm -> after_stop(.., self.reconnect_pending) (1229-1231); Disconnect evaluates after_stop with reconnect_pending read at handler entry (1125); Error arm (1215-1228): let retry = retry_after_error(self.session_target, was_stopping, left) && self.schedule_auto_reconnect(&sender); if !retry { self.session_target = None; release if marker present }",
        "AMENDMENT: the codemap site notes that say 'replace the bool with Option<ConnectionNodeRef>' and 'the Disconnect ConnectToNode sends while running must not clear the target' describe HEAD only — the codeTasks govern: the field is Option<SessionTarget { node, established }>, and that internal Disconnect does clear it (the pending replay sets a fresh target)."
      ],
      "coder": "rust-coder",
      "verify": "make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "App fields direct_connect_in_flight / pending_direct_target",
          "anchor": "direct_connect_in_flight: bool,",
          "lines": "85-86",
          "change": "replace bool with a session target Option<ConnectionNodeRef> (no ConnectionTarget type exists); pending_direct_target stays for the replay path"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "App init",
          "anchor": "direct_connect_in_flight: false,",
          "lines": "893",
          "change": "initialise session target to None"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::ConnectToNode handler (write)",
          "anchor": "self.direct_connect_in_flight = true;",
          "lines": "1070-1124",
          "change": "set session target = Some(target) after start_connection Ok; with-handle branch 1107-1112 sets pending_direct_target and sends Disconnect"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::Disconnect handler",
          "anchor": "match disconnect_plan(self.process_handle.is_some(), self.tun_marker_present()) {",
          "lines": "1125-1145",
          "change": "clear session target on user Disconnect; Disconnect is also sent internally by ApplyAndRestart (1380) and ConnectToNode-with-handle (1111), which must not clear it"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "ProcessStateConnection terminal/Running arms (reads+writes)",
          "anchor": "let retry = if self.direct_connect_in_flight {",
          "lines": "1210-1233",
          "change": "Running arm clears flag at 1213 (must keep target); Error arm 1216-1218 reads flag to skip auto-reconnect and then releases kill-switch via release_tun_session at 1225-1227 when marker present; Stopped arm clears at 1230"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "consume_terminal_direct_state call",
          "anchor": "&mut self.direct_connect_in_flight,",
          "lines": "1193-1202",
          "change": "call site passes the bool by &mut; replay sends ConnectToNode(target) after cancel_auto_reconnect"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "consume_terminal_direct_state",
          "anchor": "fn consume_terminal_direct_state(",
          "lines": "1461-1490",
          "change": "pure helper; param direct_connect_in_flight: &mut bool; Stopped always clears it, Error clears only with a pending target; NEW tests for set / keep across Running / clear on Disconnect"
        },
        {
          "task": "2.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "tests error_replays_pending_direct_target_before_any_other_reconnect / stopped_replays_pending_direct_target_before_any_other_reconnect",
          "anchor": "fn error_replays_pending_direct_target_before_any_other_reconnect() {",
          "lines": "1575-1591; stopped variant 1607-1620",
          "change": "no edit; both call consume_terminal_direct_state(&state, &mut bool, &mut Option<ConnectionNodeRef>) and assert the bool; replacing the bool param breaks them (also direct_error_leaves_in_flight_for_caller_suppression 1559, stopped_clears_stale_direct_in_flight_when_nothing_pending 1594)"
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
      "prev": "reconnect-yields",
      "sharedPkg": "crates/ui",
      "targetedTests": [
        "app::tests::direct_session_starts_unestablished",
        "app::tests::session_target_kept_across_running_and_established",
        "app::tests::session_target_kept_on_stop_during_restart",
        "app::tests::session_target_cleared_on_stop_without_restart",
        "app::tests::failed_direct_connect_gets_no_retry",
        "app::tests::established_direct_session_retries_within_budget",
        "app::tests::planned_error_retry_unchanged",
        "app::tests::error_replays_pending_direct_target_before_any_other_reconnect",
        "app::tests::stopped_replays_pending_direct_target_before_any_other_reconnect",
        "app::tests::error_without_pending_target_replays_nothing",
        "app::tests::stopped_without_pending_target_replays_nothing",
        "app::tests::error_while_stopping_releases_without_retry",
        "app::tests::error_with_reconnects_left_keeps_killswitch"
      ]
    },
    {
      "id": "restart-via-target",
      "taskIds": [
        "2.2"
      ],
      "seam": "restart-via-target",
      "contract": {
        "states": [
          "reconnect message",
          "session_target"
        ],
        "transitions": [
          {
            "input": "Stopped|Error with reconnect_pending, session_target Some",
            "state": "reconnect message",
            "effect": "set",
            "evidence": "spec: Apply and restart keeps a directly chosen node"
          },
          {
            "input": "Stopped|Error with reconnect_pending, session_target None",
            "state": "reconnect message",
            "effect": "set",
            "evidence": "design non-goal: planned sessions replan"
          },
          {
            "input": "AutoReconnect(current g), session_target Some",
            "state": "reconnect message",
            "effect": "set",
            "evidence": "spec: Automatic reconnect keeps the chosen node"
          },
          {
            "input": "ConnectToNode(t, Restart|AutoReconnect) unresolvable",
            "state": "session_target",
            "effect": "forced",
            "evidence": "spec: Directly chosen node no longer available -> notify, configured strategy"
          },
          {
            "input": "ConnectToNode(t, User) unresolvable",
            "state": "session_target",
            "effect": "no-op",
            "evidence": "app.rs:1100-1105"
          }
        ],
        "forbidden": [
          "multi-candidate plan for a restart while the direct target resolves",
          "planner fallback on a User origin",
          "fallback Connect(origin) losing the AutoReconnect origin"
        ],
        "seeding": [
          "pure fn inputs; SessionTarget via direct_session; AppMsg asserted with matches!"
        ],
        "budgets": [
          "AUTO_RECONNECT_DELAY = 5s",
          "MAX_AUTO_RECONNECTS = 3",
          "make test-ui <= 5m"
        ],
        "names": [
          "ConnectOrigin::Restart",
          "reconnect_msg",
          "falls_back_to_planner",
          "AppMsg::ConnectToNode",
          "AppMsg::Connect"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "Tests: matches! on reconnect_msg(Some(t), Restart|AutoReconnect) -> ConnectToNode(t.node, origin), None -> Connect(origin); falls_back_to_planner User false, Restart/AutoReconnect true; cancels_auto_reconnect(Restart) true",
        "Code: ConnectOrigin::Restart; fn reconnect_msg(target: Option<SessionTarget>, origin: ConnectOrigin) -> AppMsg; fn falls_back_to_planner(origin: ConnectOrigin) -> bool",
        "Code: app.rs:1206-1208 sends reconnect_msg(self.session_target, ConnectOrigin::Restart); app.rs:1237-1238 sends reconnect_msg(self.session_target, ConnectOrigin::AutoReconnect)",
        "Code: ConnectToNode resolve failure (app.rs:1100-1105): if falls_back_to_planner(origin) { toast 'Chosen node is unavailable, reconnecting with the configured strategy'; self.session_target = None; sender.input(AppMsg::Connect(origin)) } else HEAD toast",
        "AMENDMENT: restart_origin_cancels_like_user also asserts direct_session(node, ConnectOrigin::Restart).established == false, so a restart attempt that fails is not retried and releases the kill-switch."
      ],
      "coder": "rust-coder",
      "verify": "make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::ApplyAndRestart",
          "anchor": "self.reconnect_pending = true;",
          "lines": "1377-1381",
          "change": "sets reconnect_pending then Disconnect; reconnect goes through the Stopped dispatch below"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "reconnect_after_stop dispatch",
          "anchor": "if reconnect_after_stop(&state, self.reconnect_pending) {",
          "lines": "1206-1208",
          "change": "sends AppMsg::Connect; route to ConnectToNode(session target) when set"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::AutoReconnect handler",
          "anchor": "if generation == self.reconnect_generation && self.process_handle.is_none() {",
          "lines": "1236-1240",
          "change": "sends AppMsg::Connect; route via session target when set; note ConnectToNode calls cancel_auto_reconnect (1108/1115) which resets auto_reconnect_attempts, so routing auto-reconnect through it as-is would reset the 3-attempt budget"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "ConnectToNode unavailable-target toast",
          "anchor": "Node not available or disabled",
          "lines": "1100-1105",
          "change": "toast exists and returns; fallback to AppMsg::Connect needed when reached from ApplyAndRestart/AutoReconnect; resolve_candidate is crates/core/src/resolve.rs:218"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::schedule_auto_reconnect / release_on_error",
          "anchor": "fn schedule_auto_reconnect(&mut self, sender: &ComponentSender<Self>) -> bool {",
          "lines": "224-235; release_on_error 1457-1459",
          "change": "direct sessions currently never reach schedule_auto_reconnect (Error arm 1216); spec requires auto-reconnect to the chosen node after crash give-up yet immediate surfacing of a direct connect failure; which Error triggers retry vs kill-switch release is a design call"
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
      "prev": "session-target",
      "sharedPkg": "crates/ui",
      "targetedTests": [
        "app::tests::restart_reconnects_direct_session_to_its_node",
        "app::tests::restart_without_direct_session_replans",
        "app::tests::auto_reconnect_keeps_direct_node",
        "app::tests::unavailable_target_falls_back_only_for_system_reconnects",
        "app::tests::restart_origin_cancels_like_user"
      ]
    },
    {
      "id": "last-success-owned",
      "taskIds": [
        "3.1"
      ],
      "seam": "last-success-owned",
      "contract": {
        "states": [
          "settings.last_success"
        ],
        "transitions": [
          {
            "input": "FlushSettings(dialog copy)",
            "state": "last_success",
            "effect": "no-op",
            "evidence": "spec: Preferences opened before a connect"
          },
          {
            "input": "Running with ConnectionMetadata",
            "state": "last_success",
            "effect": "set",
            "evidence": "app.rs:1166-1179"
          }
        ],
        "forbidden": [
          "persisted last_success taken from the dialog copy"
        ],
        "seeding": [
          "AppSettings default with last_success = Some(LastSuccessMetadata { node_ref, connected_at })"
        ],
        "budgets": [
          "make test-ui <= 5m"
        ],
        "names": [
          "keep_last_success",
          "LastSuccessMetadata",
          "AppSettings::last_success"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "Tests: app::tests::flush_keeps_current_last_success (incoming last_success = stale A, current = B -> result B); flush_does_not_invent_last_success (incoming Some(A), current None -> None); flush_keeps_other_fields (incoming socks_port changed survives)",
        "Code: fn keep_last_success(incoming: AppSettings, current: &AppSettings) -> AppSettings; FlushSettings persists keep_last_success(settings, &self.settings)",
        "AMENDMENT (write this test first, with the other tests of this chunk): app::tests::direct_session_success_records_last_success — the last-success value written on Running for a session started by ConnectToNode names that node (extract the Running-arm last-success builder as a pure fn if it is not one already)."
      ],
      "coder": "rust-coder",
      "verify": "make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "3.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::FlushSettings",
          "anchor": "AppMsg::FlushSettings(settings) => {",
          "lines": "978-1006",
          "change": "overwrite incoming settings.last_success with self.settings.last_success before persist_settings (289-295, which replaces self.settings wholesale)"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "SettingsChanged debounce",
          "anchor": "move || s.input(AppMsg::FlushSettings(settings)),",
          "lines": "968-977",
          "change": "reference only: 300 ms debounce carries the dialog copy into FlushSettings"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "last_success write on connection metadata",
          "anchor": "settings.last_success = Some(LastSuccessMetadata {",
          "lines": "1166-1179",
          "change": "reference only: sole last_success writer; clones self.settings and persists; readers are ConnectionPlanner::new at 411 and 1053"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/preferences/mod.rs",
          "symbol": "show_preferences settings copy / emit",
          "anchor": "let settings_state = Rc::new(RefCell::new(settings.clone()));",
          "lines": "40; emit 95-100",
          "change": "reference only: copy taken at open (app.rs:1271), whole struct emitted per change; dialog reused while open (app.rs:1266); no edit if fix lands in FlushSettings"
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
      "prev": "restart-via-target",
      "sharedPkg": "crates/ui",
      "targetedTests": [
        "app::tests::flush_keeps_current_last_success",
        "app::tests::flush_does_not_invent_last_success",
        "app::tests::flush_keeps_other_fields",
        "app::tests::direct_session_success_records_last_success"
      ]
    },
    {
      "id": "log-generation",
      "taskIds": [
        "4.1"
      ],
      "seam": "log-generation",
      "contract": {
        "states": [
          "logs page contents"
        ],
        "transitions": [
          {
            "input": "ProcessLogLine(g == connection_generation)",
            "state": "logs",
            "effect": "set",
            "evidence": "app.rs:1248-1250"
          },
          {
            "input": "ProcessLogLine(g != connection_generation)",
            "state": "logs",
            "effect": "no-op",
            "evidence": "design: drops mismatches as for state messages app.rs:1150"
          },
          {
            "input": "failover to next candidate",
            "state": "generation",
            "effect": "no-op",
            "evidence": "same request, connection.rs:286-288"
          }
        ],
        "forbidden": [
          "a line from a superseded generation appended after start_connection bumped connection_generation (app.rs:486)"
        ],
        "seeding": [
          "connection test via spawn with GENERATION = 7 (connection.rs:434); app test via pure fn"
        ],
        "budgets": [
          "RECV_TIMEOUT 20s per receive",
          "make test-ui <= 5m"
        ],
        "names": [
          "AppMsg::ProcessLogLine(u64, String)",
          "is_current_generation",
          "connection_generation",
          "GENERATION"
        ]
      },
      "redTasks": [],
      "redTests": [],
      "codeTasks": [
        "Test connection::tests::log_lines_carry_connection_generation: stub '[ \"$1\" = check ] && exit 0; while :; do echo v2rs-log-line; sleep 0.2; done' with singbox_settings(); receive until AppMsg::ProcessLogLine(g, line) with line == v2rs-log-line; assert g == GENERATION; handle.stop()",
        "Test app::tests::interleaved_generations_keep_only_current: [(6, old), (7, new), (6, late)] filtered by is_current_generation(g, 7) -> [new]",
        "Code: AppMsg::ProcessLogLine(u64, String); connection.rs:249 emits ProcessLogLine(generation, line.content); fn is_current_generation(message: u64, current: u64) -> bool used at app.rs:1150 and in the ProcessLogLine arm before LogsMsg::AppendLine",
        "AMENDMENT: CHANGELOG.md [Unreleased] Fixed (Keep a Changelog, match existing entry style), one line each: Apply & Restart and automatic reconnect keep a directly chosen node; editing TUN, idle-timeout or heartbeat settings while connected raises the restart banner; saving Preferences no longer rolls back the last successful node; a manual Connect cancels a pending automatic reconnect; log lines from a replaced connection no longer appear."
      ],
      "coder": "rust-coder",
      "verify": "make test-ui && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings",
      "sites": [
        {
          "task": "4.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::ProcessLogLine variant + handler",
          "anchor": "AppMsg::ProcessLogLine(line) => {",
          "lines": "variant 106; handler 1248-1250",
          "change": "variant becomes ProcessLogLine(u64, String); handler drops when generation != self.connection_generation, mirroring the check at 1150"
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn: log_forwarder",
          "anchor": "log_sender.emit(AppMsg::ProcessLogLine(line.content));",
          "lines": "243-256",
          "change": "emit ProcessLogLine(generation, line.content); generation (u64, Copy) already in scope from ConnectionRequest; log_forwarder is halted only at 287, not on the Stop returns at 262/278"
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
      "prev": "last-success-owned",
      "sharedPkg": "crates/ui",
      "targetedTests": [
        "connection::tests::log_lines_carry_connection_generation",
        "app::tests::interleaved_generations_keep_only_current"
      ]
    },
    {
      "id": "verification",
      "taskIds": [
        "5.1",
        "5.2"
      ],
      "seam": "verification",
      "contract": {
        "states": [],
        "transitions": [],
        "forbidden": [],
        "seeding": [
          "5.1: the floor runs on the integration worktree after every chunk has closed",
          "5.2: a live app session driven by hand after merge"
        ],
        "budgets": [
          "full floor <= 10m"
        ],
        "names": []
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
          "anchor": "$(TEST) --workspace --all-targets $(CARGO_FLAGS) $(TEST_ARGS)",
          "lines": "12-17, 108-118",
          "change": "runner: make test TEST_TIMEOUT=10m (expands to timeout 10m cargo test --workspace --all-targets -- --test-threads=4); per-crate make test-core / make test-ui"
        },
        {
          "task": "5.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "live check",
          "anchor": "AppMsg::ApplyAndRestart => {",
          "lines": "1377",
          "change": "manual live run, no automated runner; launch command not verified"
        }
      ],
      "pkgDirs": [],
      "pkgs": [],
      "parallel": false,
      "shard": "",
      "prev": "log-generation",
      "sharedPkg": "workspace",
      "targetedTests": []
    }
  ],
  "seams": [
    {
      "id": "snapshot-fields",
      "tasks": [
        "1.1"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. Snapshot gains tun, idle_timeout_secs, ws_heartbeat_secs: captured, compared, restored.",
      "contract": {
        "states": [
          "snapshot.tun",
          "snapshot.idle_timeout_secs",
          "snapshot.ws_heartbeat_secs"
        ],
        "transitions": [
          {
            "input": "any TunConfig field in settings differs from snapshot",
            "state": "diverges_from",
            "effect": "set",
            "evidence": "process-lifecycle spec: TUN edit raises the restart banner"
          },
          {
            "input": "settings.idle_timeout_secs differs",
            "state": "diverges_from",
            "effect": "set",
            "evidence": "config/v2ray.rs:59 connIdle reads it"
          },
          {
            "input": "settings.ws_heartbeat_secs differs",
            "state": "diverges_from",
            "effect": "set",
            "evidence": "config/xray.rs:110 apply_ws_heartbeat"
          },
          {
            "input": "all three equal",
            "state": "diverges_from",
            "effect": "no-op",
            "evidence": "runtime_snapshot.rs:136"
          },
          {
            "input": "restore_settings",
            "state": "settings.tun/idle/heartbeat",
            "effect": "forced",
            "evidence": "runtime_snapshot.rs:45-54"
          },
          {
            "input": "start_connection",
            "state": "snapshot fields",
            "effect": "set",
            "evidence": "app.rs:489-502"
          }
        ],
        "forbidden": [
          "a fixture baseline whose tun/idle/heartbeat differ from AppSettings::default() while the test asserts no divergence"
        ],
        "seeding": [
          "RuntimeConfigSnapshot literal via make_snapshot plus struct update (..make_snapshot(..)); settings mutated field by field from AppSettings default"
        ],
        "budgets": [
          "make test-core wall clock <= 5m (Makefile TEST_TIMEOUT)"
        ],
        "names": [
          "RuntimeConfigSnapshot::tun",
          "RuntimeConfigSnapshot::idle_timeout_secs",
          "RuntimeConfigSnapshot::ws_heartbeat_secs",
          "AppSettings::tun",
          "AppSettings::idle_timeout_secs",
          "AppSettings::ws_heartbeat_secs",
          "TunConfig::interface_name",
          "TunConfig::strict_route",
          "TunConfig::enabled"
        ]
      },
      "codeTasks": [
        "Tests in crates/core/src/runtime_snapshot.rs: make_snapshot and every RuntimeConfigSnapshot literal (lines 79, 147, 220, 240, 321) get tun: TunConfig::default(), idle_timeout_secs: AppSettings::default().idle_timeout_secs, ws_heartbeat_secs: 0 so existing no-divergence asserts keep holding",
        "Add test_runtime_config_snapshot_detects_tun_divergence (settings.tun.enabled, then interface_name, then strict_route each flips diverges_from to true from a matching baseline), test_runtime_config_snapshot_detects_idle_timeout_divergence, test_runtime_config_snapshot_detects_ws_heartbeat_divergence, test_runtime_config_snapshot_restores_tun_and_timeouts",
        "Code: add pub tun: TunConfig, pub idle_timeout_secs: u32, pub ws_heartbeat_secs: u32 to RuntimeConfigSnapshot; diverges_from adds self.tun != settings.tun || self.idle_timeout_secs != settings.idle_timeout_secs || self.ws_heartbeat_secs != settings.ws_heartbeat_secs; restore_settings writes all three",
        "Code: crates/ui/src/app.rs:489-502 capture tun: self.settings.tun.clone(), idle_timeout_secs, ws_heartbeat_secs from self.settings",
        "AMENDMENT: runtime_snapshot.rs test literal at `..make_snapshot` is a struct-update literal and needs no edit; only the full RuntimeConfigSnapshot literals need the new fields."
      ]
    },
    {
      "id": "tun-from-snapshot",
      "tasks": [
        "1.2"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. Marker write and tray already satisfied (see blockers). Remaining: probe suppression at app.rs:153-157 reads edited settings.",
      "contract": {
        "states": [
          "tun_active"
        ],
        "transitions": [
          {
            "input": "Running, snapshot.tun.enabled, backend Xray|SingBox",
            "state": "tun_active",
            "effect": "set",
            "evidence": "app.rs:153-157"
          },
          {
            "input": "Running, snapshot.tun disabled, settings.tun enabled mid-session",
            "state": "tun_active",
            "effect": "clear",
            "evidence": "spec scenario: TUN indicators follow the running session"
          },
          {
            "input": "Running, snapshot backend V2ray",
            "state": "tun_active",
            "effect": "clear",
            "evidence": "app.rs:155"
          },
          {
            "input": "any non-Running state",
            "state": "tun_active",
            "effect": "clear",
            "evidence": "app.rs:153"
          },
          {
            "input": "Running with runtime_snapshot None",
            "state": "tun_active",
            "effect": "clear",
            "evidence": "clear_restart_flow app.rs:210-213 may run before a late relay"
          },
          {
            "input": "settings.tun edited mid-session",
            "state": "tun_session.json",
            "effect": "no-op",
            "evidence": "connection.rs:172-179 reads launched settings"
          }
        ],
        "forbidden": [
          "tun_active reading self.settings while Running"
        ],
        "seeding": [
          "pure fn inputs: RuntimeConfigSnapshot literal in app tests; marker via the real connection task with a stub backend (connection.rs:443-493 helpers)"
        ],
        "budgets": [
          "connection test RECV_TIMEOUT 20s (connection.rs:435); make test-ui <= 5m"
        ],
        "names": [
          "tun_active_for",
          "SubscriptionsMsg::SetTunActive",
          "load_tun_session",
          "singbox_settings"
        ]
      },
      "codeTasks": [
        "Test app::tests::tun_active_follows_launched_snapshot: snapshot with tun disabled -> false; tun enabled + Xray -> true; tun enabled + V2ray -> false (all with ProcessState::Running)",
        "Test app::tests::tun_active_false_outside_running: Starting/Stopping/Stopped/Error with a TUN snapshot -> false; Running with None -> false",
        "Test connection::tests::no_marker_when_launched_without_tun: stub 'exit 1', singbox_settings() (tun disabled), one candidate; wait for Error; assert load_tun_session(&stub.paths) is None",
        "Code: fn tun_active_for(state: &ProcessState, snapshot: Option<&RuntimeConfigSnapshot>) -> bool = Running && snapshot.is_some_and(|s| s.tun.enabled && s.backend_type != BackendType::V2ray); apply_state uses it with self.runtime_snapshot.as_ref()"
      ]
    },
    {
      "id": "reconnect-yields",
      "tasks": [
        "3.2"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. Disconnect and valid ConnectToNode already cancel (blockers). ConnectOrigin on the message: user Connect cancels; AutoReconnect-originated connects do not, else the auto path resets the budget.",
      "contract": {
        "states": [
          "reconnect_generation",
          "auto_reconnect_attempts"
        ],
        "transitions": [
          {
            "input": "Connect(User), no handle, no release in flight",
            "state": "reconnect_generation/attempts",
            "effect": "clear",
            "evidence": "spec: Manual connect during the reconnect delay"
          },
          {
            "input": "Connect(AutoReconnect)",
            "state": "reconnect_generation/attempts",
            "effect": "no-op",
            "evidence": "design: unless the message came from AutoReconnect"
          },
          {
            "input": "Connect(*) with handle or tun_release_in_flight",
            "state": "reconnect_generation/attempts",
            "effect": "no-op",
            "evidence": "app.rs:1021-1023"
          },
          {
            "input": "ConnectToNode(t, User) resolvable",
            "state": "reconnect_generation/attempts",
            "effect": "clear",
            "evidence": "app.rs:1108,1115"
          },
          {
            "input": "ConnectToNode(t, User) unresolvable",
            "state": "reconnect_generation/attempts",
            "effect": "no-op",
            "evidence": "app.rs:1074-1076"
          },
          {
            "input": "ConnectToNode(t, AutoReconnect)",
            "state": "reconnect_generation/attempts",
            "effect": "no-op",
            "evidence": "budget must count toward MAX_AUTO_RECONNECTS"
          },
          {
            "input": "Disconnect",
            "state": "reconnect_generation/attempts",
            "effect": "clear",
            "evidence": "app.rs:1127 (satisfied)"
          },
          {
            "input": "AutoReconnect(g), g == current, no handle",
            "state": "fires",
            "effect": "set",
            "evidence": "app.rs:1237"
          },
          {
            "input": "AutoReconnect(g), g != current",
            "state": "fires",
            "effect": "no-op",
            "evidence": "app.rs:1237"
          }
        ],
        "forbidden": [
          "an AutoReconnect-originated connect resetting auto_reconnect_attempts",
          "a user Connect followed by the old timer firing Connect"
        ],
        "seeding": [
          "generations are plain u32 inputs to auto_reconnect_fires; cancellation modeled as wrapping_add(1) exactly as cancel_auto_reconnect (app.rs:217-220)"
        ],
        "budgets": [
          "AUTO_RECONNECT_DELAY = 5s (app.rs:36)",
          "MAX_AUTO_RECONNECTS = 3 (app.rs:35)",
          "make test-ui <= 5m"
        ],
        "names": [
          "ConnectOrigin",
          "ConnectOrigin::User",
          "ConnectOrigin::AutoReconnect",
          "AppMsg::Connect(ConnectOrigin)",
          "AppMsg::ConnectToNode(ConnectionNodeRef, ConnectOrigin)",
          "cancels_auto_reconnect",
          "auto_reconnect_fires",
          "cancel_auto_reconnect",
          "reconnect_generation"
        ]
      },
      "codeTasks": [
        "Tests: cancels_auto_reconnect(User) true, (AutoReconnect) false; auto_reconnect_fires(g, g.wrapping_add(1), false) false, (g, g, false) true, (g, g, true) false",
        "Code: #[derive(Debug, Clone, Copy, PartialEq, Eq)] enum ConnectOrigin { User, AutoReconnect }; AppMsg::Connect(ConnectOrigin); AppMsg::ConnectToNode(ConnectionNodeRef, ConnectOrigin)",
        "Code: fn cancels_auto_reconnect(origin: ConnectOrigin) -> bool (origin != AutoReconnect); fn auto_reconnect_fires(message: u32, current: u32, has_handle: bool) -> bool extracted from app.rs:1237",
        "Code: Connect handler calls self.cancel_auto_reconnect() right after the handle/tun_release_in_flight guard (app.rs:1021-1023) when cancels_auto_reconnect(origin); ConnectToNode guards its two existing cancel calls with the same fn",
        "Code: construct sites: app.rs:767 tray and 1017 toggle -> Connect(User); 812/826 page forwards and 1200 pending replay -> ConnectToNode(_, User); 1208 -> Connect(User) (restart-via-target replaces it); 1238 -> Connect(AutoReconnect)",
        "AMENDMENT (supersedes the cancel placement above): in the Connect handler, call cancel_auto_reconnect() only after the candidates.is_empty() check, immediately before start_connection — a user Connect that fails to load stores, finds no candidates, or never reaches start_connection leaves the timer armed, the same rule as an unresolvable ConnectToNode.",
        "AMENDMENT: test binding — user_connect_cancels_pending_auto_reconnect asserts cancels_auto_reconnect(ConnectOrigin::User) is true; auto_reconnect_connect_keeps_budget asserts cancels_auto_reconnect(ConnectOrigin::AutoReconnect) is false; auto_reconnect_fires_only_for_current_generation_without_handle asserts auto_reconnect_fires(g, g.wrapping_add(1), false) is false, (g, g, false) is true, (g, g, true) is false. auto_reconnect_exhausted_after_three_attempts already exists and must stay green."
      ]
    },
    {
      "id": "session-target",
      "tasks": [
        "2.1",
        "2.3"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. direct_connect_in_flight -> session_target: Option<SessionTarget> (no ConnectionTarget type exists). established keeps harden behavior: unestablished direct failure gets no retry and releases; established session retries within budget, kill-switch held while scheduled.",
      "contract": {
        "states": [
          "session_target",
          "session_target.established",
          "tun_session.json (kill-switch)"
        ],
        "transitions": [
          {
            "input": "ConnectToNode(t, User|Restart) start Ok",
            "state": "session_target",
            "effect": "set",
            "evidence": "design: set by ConnectToNode; established=false"
          },
          {
            "input": "ConnectToNode(t, AutoReconnect) start Ok",
            "state": "session_target",
            "effect": "set",
            "evidence": "established=true, auto path only follows an established session"
          },
          {
            "input": "ConnectToNode start_connection Err",
            "state": "session_target",
            "effect": "clear",
            "evidence": "app.rs:1118-1123"
          },
          {
            "input": "ConnectToNode(t, User) unresolvable",
            "state": "session_target",
            "effect": "no-op",
            "evidence": "app.rs:1074-1076"
          },
          {
            "input": "Connect(*) reaching start_connection",
            "state": "session_target",
            "effect": "clear",
            "evidence": "planned session replaces direct; spec: ordinary connects use the strategy"
          },
          {
            "input": "Running",
            "state": "session_target.established",
            "effect": "set",
            "evidence": "tasks 2.1 keep across Running"
          },
          {
            "input": "Stopped with reconnect_pending",
            "state": "session_target",
            "effect": "no-op",
            "evidence": "ApplyAndRestart app.rs:1377-1381"
          },
          {
            "input": "Stopped without reconnect_pending",
            "state": "session_target",
            "effect": "clear",
            "evidence": "app.rs:1229-1231"
          },
          {
            "input": "user Disconnect (reconnect_pending false at entry)",
            "state": "session_target",
            "effect": "clear",
            "evidence": "tasks 2.1 clear on Disconnect"
          },
          {
            "input": "Disconnect sent by ApplyAndRestart (reconnect_pending true)",
            "state": "session_target",
            "effect": "no-op",
            "evidence": "app.rs:1379-1380"
          },
          {
            "input": "Error, target unestablished",
            "state": "session_target + kill-switch",
            "effect": "clear",
            "evidence": "app.rs:1216-1218,1225-1227; spec: direct connect failure surfaces immediately"
          },
          {
            "input": "Error, target established, !release_on_error, schedule true",
            "state": "session_target + kill-switch",
            "effect": "no-op",
            "evidence": "spec: automatic reconnect keeps the chosen node; harden release_on_error app.rs:1457"
          },
          {
            "input": "Error, target established, stopping or budget spent or pending_exit",
            "state": "session_target + kill-switch",
            "effect": "clear",
            "evidence": "app.rs:1225-1227"
          },
          {
            "input": "Error, None",
            "state": "kill-switch",
            "effect": "no-op",
            "evidence": "planned path unchanged app.rs:1220-1223"
          },
          {
            "input": "Stopped|Error with pending_direct_target",
            "state": "pending_direct_target",
            "effect": "clear",
            "evidence": "app.rs:1193-1201, replay sets a new session_target"
          }
        ],
        "forbidden": [
          "retry scheduled for an unestablished direct target",
          "tun marker released while a retry is scheduled",
          "ApplyAndRestart's Disconnect clearing session_target",
          "established=true reached except through Running or an AutoReconnect ConnectToNode"
        ],
        "seeding": [
          "pure fn inputs only; Some(SessionTarget{established:false}) via direct_session(node, ConnectOrigin::User); established=true via session_target_after_running(Some(direct_session(..))); node = ConnectionNodeRef::Manual { node_id: uuid::Uuid::nil() }"
        ],
        "budgets": [
          "MAX_AUTO_RECONNECTS = 3 retries for an established direct session",
          "make test-ui <= 5m"
        ],
        "names": [
          "SessionTarget",
          "SessionTarget::node",
          "SessionTarget::established",
          "App::session_target",
          "direct_session",
          "session_target_after_running",
          "session_target_after_stop",
          "retry_after_error",
          "release_on_error",
          "consume_terminal_direct_state",
          "pending_direct_target"
        ]
      },
      "codeTasks": [
        "Tests (names in targetedTests): direct_session User -> established false, AutoReconnect -> true; after_running sets established; after_stop(t,true)=t, (t,false)=None (covers user Disconnect); retry_after_error: unestablished -> false, established or None -> !release_on_error(stopping, left)",
        "Tests 2.3: keep error_/stopped_replays_pending_direct_target_before_any_other_reconnect names and replay/pending asserts, drop in-flight arg; the two in-flight-only tests become error_/stopped_without_pending_target_replays_nothing (blockers)",
        "Code: #[derive(Debug, Clone, Copy, PartialEq, Eq)] struct SessionTarget { node: ConnectionNodeRef, established: bool }; App field session_target replaces direct_connect_in_flight (app.rs:86, 893)",
        "Code: fn direct_session(node, origin) -> SessionTarget; fn session_target_after_running(Option<SessionTarget>) -> Option<SessionTarget>; fn session_target_after_stop(Option<SessionTarget>, reconnect_pending: bool) -> Option<SessionTarget>; fn retry_after_error(target: Option<SessionTarget>, app_state_stopping: bool, reconnects_left: u32) -> bool; consume_terminal_direct_state(state, &mut Option<ConnectionNodeRef>) -> Option<ConnectionNodeRef>",
        "Code wiring: ConnectToNode start Ok -> Some(direct_session(target, origin)), Err -> None (app.rs:1118-1123); Connect sets None when it calls start_connection; Running arm -> after_running (1213); Stopped arm -> after_stop(.., self.reconnect_pending) (1229-1231); Disconnect evaluates after_stop with reconnect_pending read at handler entry (1125); Error arm (1215-1228): let retry = retry_after_error(self.session_target, was_stopping, left) && self.schedule_auto_reconnect(&sender); if !retry { self.session_target = None; release if marker present }",
        "AMENDMENT: the codemap site notes that say 'replace the bool with Option<ConnectionNodeRef>' and 'the Disconnect ConnectToNode sends while running must not clear the target' describe HEAD only — the codeTasks govern: the field is Option<SessionTarget { node, established }>, and that internal Disconnect does clear it (the pending replay sets a fresh target)."
      ]
    },
    {
      "id": "restart-via-target",
      "tasks": [
        "2.2"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. Restart and AutoReconnect route through reconnect_msg(session_target, origin); unresolvable target on a system origin toasts, clears, falls back to Connect(origin).",
      "contract": {
        "states": [
          "reconnect message",
          "session_target"
        ],
        "transitions": [
          {
            "input": "Stopped|Error with reconnect_pending, session_target Some",
            "state": "reconnect message",
            "effect": "set",
            "evidence": "spec: Apply and restart keeps a directly chosen node"
          },
          {
            "input": "Stopped|Error with reconnect_pending, session_target None",
            "state": "reconnect message",
            "effect": "set",
            "evidence": "design non-goal: planned sessions replan"
          },
          {
            "input": "AutoReconnect(current g), session_target Some",
            "state": "reconnect message",
            "effect": "set",
            "evidence": "spec: Automatic reconnect keeps the chosen node"
          },
          {
            "input": "ConnectToNode(t, Restart|AutoReconnect) unresolvable",
            "state": "session_target",
            "effect": "forced",
            "evidence": "spec: Directly chosen node no longer available -> notify, configured strategy"
          },
          {
            "input": "ConnectToNode(t, User) unresolvable",
            "state": "session_target",
            "effect": "no-op",
            "evidence": "app.rs:1100-1105"
          }
        ],
        "forbidden": [
          "multi-candidate plan for a restart while the direct target resolves",
          "planner fallback on a User origin",
          "fallback Connect(origin) losing the AutoReconnect origin"
        ],
        "seeding": [
          "pure fn inputs; SessionTarget via direct_session; AppMsg asserted with matches!"
        ],
        "budgets": [
          "AUTO_RECONNECT_DELAY = 5s",
          "MAX_AUTO_RECONNECTS = 3",
          "make test-ui <= 5m"
        ],
        "names": [
          "ConnectOrigin::Restart",
          "reconnect_msg",
          "falls_back_to_planner",
          "AppMsg::ConnectToNode",
          "AppMsg::Connect"
        ]
      },
      "codeTasks": [
        "Tests: matches! on reconnect_msg(Some(t), Restart|AutoReconnect) -> ConnectToNode(t.node, origin), None -> Connect(origin); falls_back_to_planner User false, Restart/AutoReconnect true; cancels_auto_reconnect(Restart) true",
        "Code: ConnectOrigin::Restart; fn reconnect_msg(target: Option<SessionTarget>, origin: ConnectOrigin) -> AppMsg; fn falls_back_to_planner(origin: ConnectOrigin) -> bool",
        "Code: app.rs:1206-1208 sends reconnect_msg(self.session_target, ConnectOrigin::Restart); app.rs:1237-1238 sends reconnect_msg(self.session_target, ConnectOrigin::AutoReconnect)",
        "Code: ConnectToNode resolve failure (app.rs:1100-1105): if falls_back_to_planner(origin) { toast 'Chosen node is unavailable, reconnecting with the configured strategy'; self.session_target = None; sender.input(AppMsg::Connect(origin)) } else HEAD toast",
        "AMENDMENT: restart_origin_cancels_like_user also asserts direct_session(node, ConnectOrigin::Restart).established == false, so a restart attempt that fails is not retried and releases the kill-switch."
      ]
    },
    {
      "id": "last-success-owned",
      "tasks": [
        "3.1"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. FlushSettings keeps the app current last_success (app.rs:978-984).",
      "contract": {
        "states": [
          "settings.last_success"
        ],
        "transitions": [
          {
            "input": "FlushSettings(dialog copy)",
            "state": "last_success",
            "effect": "no-op",
            "evidence": "spec: Preferences opened before a connect"
          },
          {
            "input": "Running with ConnectionMetadata",
            "state": "last_success",
            "effect": "set",
            "evidence": "app.rs:1166-1179"
          }
        ],
        "forbidden": [
          "persisted last_success taken from the dialog copy"
        ],
        "seeding": [
          "AppSettings default with last_success = Some(LastSuccessMetadata { node_ref, connected_at })"
        ],
        "budgets": [
          "make test-ui <= 5m"
        ],
        "names": [
          "keep_last_success",
          "LastSuccessMetadata",
          "AppSettings::last_success"
        ]
      },
      "codeTasks": [
        "Tests: app::tests::flush_keeps_current_last_success (incoming last_success = stale A, current = B -> result B); flush_does_not_invent_last_success (incoming Some(A), current None -> None); flush_keeps_other_fields (incoming socks_port changed survives)",
        "Code: fn keep_last_success(incoming: AppSettings, current: &AppSettings) -> AppSettings; FlushSettings persists keep_last_success(settings, &self.settings)",
        "AMENDMENT (write this test first, with the other tests of this chunk): app::tests::direct_session_success_records_last_success — the last-success value written on Running for a session started by ConnectToNode names that node (extract the Running-arm last-success builder as a pure fn if it is not one already)."
      ]
    },
    {
      "id": "log-generation",
      "tasks": [
        "4.1"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests are the first codeTask; NO-TESTER-WAIVER: rust stack, chunk closes on its verify command. Tag attached in the log forwarder (connection.rs:243-256), dropped in the ProcessLogLine arm (app.rs:1248). Forwarder is not halted on Stop (connection.rs:260-265), so stale lines can follow a newer connect.",
      "contract": {
        "states": [
          "logs page contents"
        ],
        "transitions": [
          {
            "input": "ProcessLogLine(g == connection_generation)",
            "state": "logs",
            "effect": "set",
            "evidence": "app.rs:1248-1250"
          },
          {
            "input": "ProcessLogLine(g != connection_generation)",
            "state": "logs",
            "effect": "no-op",
            "evidence": "design: drops mismatches as for state messages app.rs:1150"
          },
          {
            "input": "failover to next candidate",
            "state": "generation",
            "effect": "no-op",
            "evidence": "same request, connection.rs:286-288"
          }
        ],
        "forbidden": [
          "a line from a superseded generation appended after start_connection bumped connection_generation (app.rs:486)"
        ],
        "seeding": [
          "connection test via spawn with GENERATION = 7 (connection.rs:434); app test via pure fn"
        ],
        "budgets": [
          "RECV_TIMEOUT 20s per receive",
          "make test-ui <= 5m"
        ],
        "names": [
          "AppMsg::ProcessLogLine(u64, String)",
          "is_current_generation",
          "connection_generation",
          "GENERATION"
        ]
      },
      "codeTasks": [
        "Test connection::tests::log_lines_carry_connection_generation: stub '[ \"$1\" = check ] && exit 0; while :; do echo v2rs-log-line; sleep 0.2; done' with singbox_settings(); receive until AppMsg::ProcessLogLine(g, line) with line == v2rs-log-line; assert g == GENERATION; handle.stop()",
        "Test app::tests::interleaved_generations_keep_only_current: [(6, old), (7, new), (6, late)] filtered by is_current_generation(g, 7) -> [new]",
        "Code: AppMsg::ProcessLogLine(u64, String); connection.rs:249 emits ProcessLogLine(generation, line.content); fn is_current_generation(message: u64, current: u64) -> bool used at app.rs:1150 and in the ProcessLogLine arm before LogsMsg::AppendLine",
        "AMENDMENT: CHANGELOG.md [Unreleased] Fixed (Keep a Changelog, match existing entry style), one line each: Apply & Restart and automatic reconnect keep a directly chosen node; editing TUN, idle-timeout or heartbeat settings while connected raises the restart banner; saving Preferences no longer rolls back the last successful node; a manual Connect cancels a pending automatic reconnect; log lines from a replaced connection no longer appear."
      ]
    },
    {
      "id": "verification",
      "tasks": [
        "5.1",
        "5.2"
      ],
      "summary": "NO-RED-WAIVER: verification; NO-TESTER-WAIVER: 5.1 is the floor, 5.2 is a manual live check run by the user after merge.",
      "contract": {
        "states": [],
        "transitions": [],
        "forbidden": [],
        "seeding": [
          "5.1: the floor runs on the integration worktree after every chunk has closed",
          "5.2: a live app session driven by hand after merge"
        ],
        "budgets": [
          "full floor <= 10m"
        ],
        "names": []
      },
      "codeTasks": [
        "5.1: make lint && make test TEST_TIMEOUT=10m",
        "5.2 (user, live): direct-connect node X, edit a routing rule, Apply & Restart -> status bar shows X; toggle TUN while connected -> banner appears, probes still run"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "### Requirement: Capture launched runtime snapshot The system SHALL capture an immutable snapshot of the restart-relevant settings and routing rules that were actually used for the current connection attempt. Restart-relevant settings SHALL include every setting that changes the generated backend config, including the TUN settings and the connection idle-timeout and WebSocket-heartbeat settings. While connected, UI state derived from those settings SHALL follow the snapshot rather than the edited settings.",
      "tests": [
        "runtime_snapshot::tests::test_runtime_config_snapshot_creation",
        "runtime_snapshot::tests::test_runtime_config_snapshot_detects_tun_divergence",
        "runtime_snapshot::tests::test_runtime_config_snapshot_restores_tun_and_timeouts"
      ]
    },
    {
      "shall": "- **THEN** the app SHALL keep treating the session as non-TUN — background latency probes continue and no TUN recovery marker is written — until the session is restarted with the new settings",
      "tests": [
        "app::tests::tun_active_follows_launched_snapshot",
        "connection::tests::no_marker_when_launched_without_tun"
      ]
    },
    {
      "shall": "### Requirement: Apply pending runtime changes by restart The system SHALL apply pending runtime configuration changes by reusing the normal disconnect/reconnect flow. When the running session was started by a direct connection to a chosen node, the reconnect SHALL target that node as the sole candidate.",
      "tests": [
        "app::tests::restart_reconnects_direct_session_to_its_node",
        "app::tests::restart_without_direct_session_replans"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL reconnect to that same node as the sole candidate",
      "tests": [
        "app::tests::restart_reconnects_direct_session_to_its_node"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL notify the user and reconnect using the configured strategy",
      "tests": [
        "app::tests::unavailable_target_falls_back_only_for_system_reconnects"
      ]
    },
    {
      "shall": "- **THEN** the restart-required banner SHALL appear",
      "tests": [
        "runtime_snapshot::tests::test_runtime_config_snapshot_detects_tun_divergence",
        "runtime_snapshot::tests::test_runtime_config_snapshot_detects_idle_timeout_divergence",
        "runtime_snapshot::tests::test_runtime_config_snapshot_detects_ws_heartbeat_divergence",
        "app::tests::restart_banner_only_visible_while_runtime_is_active"
      ]
    },
    {
      "shall": "### Requirement: Direct connection to a chosen node The system SHALL let the user connect directly to a specific enabled node, using that node as the only connection candidate for the attempt. The action SHALL NOT change the configured auto-resolve strategy, and subsequent ordinary connects SHALL use the configured strategy unchanged. Reconnects the system starts on the user's behalf for that session — applying pending changes and automatic reconnects after a failure — SHALL target the same node as the sole candidate.",
      "tests": [
        "app::tests::session_target_kept_across_running_and_established",
        "app::tests::auto_reconnect_keeps_direct_node",
        "app::tests::restart_reconnects_direct_session_to_its_node",
        "manual:5.2 (Connect clearing session_target is handler wiring, untested)"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL attempt the connection with that node as the sole candidate, without falling back to other nodes on failure",
      "tests": [
        "app::tests::failed_direct_connect_gets_no_retry",
        "app::tests::resolve_candidate_disabled_subscription_with_enabled_node_is_unavailable"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL stop the current session and connect to the chosen node",
      "tests": [
        "app::tests::stopped_replays_pending_direct_target_before_any_other_reconnect"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL surface the error without trying any other candidate",
      "tests": [
        "app::tests::failed_direct_connect_gets_no_retry",
        "app::tests::error_replays_pending_direct_target_before_any_other_reconnect"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL record it as the last successful node, the same as any other successful connection",
      "tests": [
        "app::tests::direct_session_success_records_last_success"
      ]
    },
    {
      "shall": "- **THEN** the reconnect SHALL target the directly chosen node as the sole candidate",
      "tests": [
        "app::tests::auto_reconnect_keeps_direct_node",
        "app::tests::established_direct_session_retries_within_budget"
      ]
    },
    {
      "shall": "### Requirement: Last-success metadata is owned by connection outcomes The last-success record SHALL change only when a connection succeeds. Saving settings from the preferences dialog SHALL NOT modify it.",
      "tests": [
        "app::tests::flush_keeps_current_last_success",
        "app::tests::flush_does_not_invent_last_success"
      ]
    },
    {
      "shall": "- **THEN** the persisted last-success record SHALL still name node B",
      "tests": [
        "app::tests::flush_keeps_current_last_success"
      ]
    },
    {
      "shall": "### Requirement: Automatic reconnect yields to the user A pending automatic reconnect SHALL be cancelled by any user-initiated Connect, direct connect, or Disconnect, so user retries neither trigger an extra immediate attempt nor consume the automatic-reconnect budget.",
      "tests": [
        "app::tests::user_connect_cancels_pending_auto_reconnect",
        "app::tests::auto_reconnect_connect_keeps_budget"
      ]
    },
    {
      "shall": "- **THEN** the scheduled reconnect SHALL NOT fire, and only the user's attempt SHALL run",
      "tests": [
        "app::tests::user_connect_cancels_pending_auto_reconnect",
        "app::tests::auto_reconnect_fires_only_for_current_generation_without_handle"
      ]
    },
    {
      "shall": "- **THEN** the scheduled reconnect SHALL NOT fire",
      "tests": [
        "app::tests::auto_reconnect_fires_only_for_current_generation_without_handle",
        "app::tests::session_target_cleared_on_stop_without_restart"
      ]
    }
  ],
  "testHarness": [
    "make_snapshot — crates/core/src/runtime_snapshot.rs:78 — RuntimeConfigSnapshot with given backend/binary, ports 1080/1081, defaults for DNS/routing/nodes, timestamp 1234567890",
    "paths_with_marker — crates/ui/src/app.rs:1761 — AppPaths (AppProfile::Test) in a TempDir with a saved TunSession {Xray, tun9}",
    "stub_helper — crates/ui/src/app.rs:1774 — executable sh script netctl stub in a TempDir",
    "Stub / stub — crates/ui/src/connection.rs:437/443 — TempDir + AppPaths (Test profile, ensure_dirs) + executable sh backend script",
    "candidate — crates/ui/src/connection.rs:457 — ConnectionCandidate for a manual node (random uuid) wrapping node(address)",
    "connect — crates/ui/src/connection.rs:469 — calls spawn() with a full ConnectionRequest (generation GENERATION=7, fresh TunLifecycle) and returns (ConnectionHandle, relm4::Receiver<AppMsg>)",
    "singbox_settings — crates/ui/src/connection.rs:495 — AppSettings::default with backend SingBox",
    "next_state — crates/ui/src/connection.rs:501 — awaits next ProcessStateConnection (20s timeout), asserts generation == GENERATION, skips other msgs",
    "assert_nothing_after_terminal — crates/ui/src/connection.rs:518 — drains until channel closes, panics on any further state msg",
    "node — crates/ui/src/connection.rs:623 — Shadowsocks ProxyNode at address:8388",
    "tun_settings — crates/ui/src/connection.rs:633 — AppSettings with tun.enabled = true",
    "create_test_subscription / create_test_node — crates/ui/src/subscriptions.rs:2210/2230 — Subscription and SubscriptionNode fixtures used by tun_active probe tests (2346, 2360)",
    "preferences tests — crates/ui/src/preferences/tun.rs:802 and dns.rs:1799 — no shared helpers; preferences/mod.rs has no test module"
  ],
  "floor": "make lint && make test TEST_TIMEOUT=10m (matches ci.yml)",
  "risks": [
    "persist-backend-diagnostics overlaps app.rs (failure toasts) and connection.rs (3.3 passes the backend log path through ConnectionRequest), plus manager.rs reader tasks -> land this change first; its ConnectionRequest field and log-file writes are independent of the ProcessLogLine tag, rebase is textual",
    "AppMsg::Connect/ConnectToNode/ProcessLogLine shape change touches every construct site -> compiler-enforced; list in reconnect-yields codeTasks",
    "Direct node that keeps crashing retries 3 x 5s before give-up -> accepted in design risks; final Error releases the kill-switch",
    "Chunk interim: between session-target and restart-via-target an established direct session auto-reconnects via the planner -> chunks land together, not merged separately",
    "CHANGELOG [Unreleased] entry not in tasks.md -> add a Fixed entry with the verification chunk"
  ],
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 1
  }
}
```
