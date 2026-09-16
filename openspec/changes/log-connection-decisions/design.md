## Context

- Exit records: `write_exit_record(requested, status)` (`crates/process/src/manager.rs:451-466`). `graceful_stop` always passes `true` (`:597-617`); it runs for every stop (`stop`, `:294`), for a launch failure after spawn (TUN device timeout `:378-381`, `xray-up` failure `:384-386`), and via `shutdown` (`:307-310`). Crash exits pass `false` (`:640`, `:324`).
- `last_output_line` picks the last non-empty stderr line, else stdout, from the last 50 buffered lines (`:725-737`); xray writes access and error lines to stdout, so a busy session ends on an access line. The same function feeds the crash `Error` message (`:625-627`), which `stop-failover-on-shared-failure` normalizes.
- Session record: `backend version node tun` (`:395-416`), written by `launch` on every start and respawn (`:359`).
- `ConnectionCmd` has a bare `Stop` (`crates/ui/src/connection.rs:54-62`); the task calls `mgr.shutdown()` on it (`:205`, `:221`, `:274`). Candidate failures are pushed silently to `failures` (`:161`, `:228`, `:287`). `resolve_effective_config` result is not logged (`:139-144`). TUN runtime fields `capture_dns`, `strict` and `nodes_pinned` are computed in `build_tun_runtime` (`:367-396`).
- App: `ConnectOrigin { User, AutoReconnect, Restart }` (`crates/ui/src/app.rs:94-102`); `ConnectToNode` carries an origin too (`:110`). Stops come from `Disconnect` (`:1165-1188`), which `ApplyAndRestart` (`:1425-1429`, `reconnect_pending`), node switch (`:1145-1152`, `pending_direct_target`) and `quit` (`:271-296`) reuse. `schedule_auto_reconnect` logs nothing (`:233-244`); `MAX_AUTO_RECONNECTS = 3`, delay 5s (`:35-36`).
- Status labels: `Starting` always renders "Connecting…" (`:198`). A crash respawn relays `Running → Starting → Running` with the same generation through the state forwarder (`connection.rs:236-254`, `manager.rs:658-662`).
- `StateManager::transition` emits events and logs nothing (`crates/process/src/state.rs:95-107`).
- No signal handling or panic hook exists in the workspace. Startup reaps an orphan via `cleanup_orphaned_backend` (`app.rs:2216-2219`, called at `:807`), whose `check_and_kill_orphaned` returns whether it killed a live orphan (`crates/process/src/pid.rs:130-178`). The PID file is removed on every clean stop and crash exit (`manager.rs:710-711`), so one present at startup means the previous run did not stop its backend.
- Evidence: `backend.log*` 2026-09-13..15 — 87 sessions (59 xray, 25 sing-box, 3 v2ray); 59 `exit requested=true`, 25 `requested=false code=1` (sing-box startup FATALs); app starts at 2026-09-14 07:11:44 and 07:16:45 UTC each ran TUN recovery with the prior session lacking an exit record.

## Goals / Non-Goals

**Goals:**
- Every exit and every connection decision can be explained from the two log files alone.

**Non-Goals:**
- Capturing `netctl recover` output or changing its timeout (`complete-tun-preflight`).
- Stripping ANSI from user-facing failure text (`stop-failover-on-shared-failure`) or from log files (`reduce-backend-log-noise`).
- Changing the crash `Error` message or failover policy.

## Decisions

- **Reason travels with the stop.** `ConnectionCmd::Stop(StopReason)` and `ProcessManager::shutdown_with(StopReason)`; the manager stores it before `graceful_stop` and `write_exit_record` prints it. Launch-failure stops set `StartFailed`; crash paths print `crash`. `StopReason` lives in `v2ray-rs-process` (the manager writes it). The app maps: `Disconnect` → `user-stop` unless `reconnect_pending` (`apply-restart`) or `pending_direct_target` (`node-switch`); `quit` → `app-quit`. Alternative rejected: inferring the reason in the app from later events — the exit record is written before the app learns anything.
- **Keep `requested=`, add `reason=`.** Existing tooling and the spec scenario key on `requested`; `reason` refines it. No `failover` or `auto-reconnect` exit reason: a candidate given up after its crash budget already exits as `crash`, and auto-reconnect only starts after the terminal `Error`; both are logged as decisions in the app log instead.
- **`last_error` beside `last_output`, not replacing it.** `last_output_line` also builds the crash `Error` text that `stop-failover-on-shared-failure` compares across candidates; changing it would couple the two changes. `last_error_line` scans the same 50 lines for `[Warning]`, `[Error]`, `WARN`, `ERROR`, `FATAL`, `panic` tokens (tolerating ANSI color codes around them).
- **Session fields supplied by the caller.** `ProcessManager::with_session_fields(String)` appended to the session line; the connection task builds `hijack=… capture_dns=… strict=… nodes_pinned=… profile=…`. Keeps TUN DNS policy out of the process crate. The effective DNS server list is `surface-effective-dns`'s separate `dns` records.
- **App log lines at `info`, one per decision.** `connect origin=… strategy=… candidates=n profile_override=bool`; `candidate start i/n label=…`; `candidate failed i/n label=… reason=…` (reason through the same single-line truncation as records); `auto-reconnect scheduled attempt=i/3 delay=5s`, `auto-reconnect exhausted`; `backend state from → to` in `StateManager::transition`; `backend exited unexpectedly (code=…); respawn crash=i/3` in `handle_unexpected_exit`; `quit requested plan=…`. `node` origin is `ConnectToNode` with `User`.
- **Signals via the GLib main loop.** `glib::unix_signal_add_local` for SIGTERM, SIGINT, SIGHUP sends `AppMsg::TrayQuit`'s quit path; a second signal while `pending_exit` is set exits immediately so a wedged stop cannot make the app unkillable. Alternative rejected: a tokio signal task — the quit path owns GTK state and must run on the main loop.
- **Panic hook chains.** Installed right after `init_logging`, logs `error` with message and location, then calls the previous hook.
- **Unclean previous run.** When `paths.pid_file_path()` exists before `cleanup_orphaned_backend` runs, append `unclean previous run left backend pid=… killed=bool` to `backend.log` through a short-lived `RotatingFileWriter`.
- **Status text from origin and transition.** The app stores the origin of the current connection and the auto-reconnect attempt; a `Starting` that follows `Running` in the same generation renders "Restarting after crash", a start with origin `AutoReconnect` renders "Reconnecting (n/3)".

## Risks / Trade-offs

- [A signal during a TUN teardown] → same `pending_exit` flow as a user quit; second signal forces exit, recovery marker still handles the next launch.
- [State transition lines add volume] → a handful per connection at `info`.
- [Keyword match for `last_error` misses a backend's wording] → field is additive; `last_output` is unchanged.

## Implementation plan

Tier `heavy`, mode `existing-service-strict`, lenses `spec`, `quality`, `arch`. Planned at `1d6a0fb8`.

Rust has no red-stage test-writer agent, so every seam carries `NO-RED-WAIVER` / `NO-TESTER-WAIVER` and its tests are the first `codeTasks` of the chunk that owns them.

This change lands third in sprint `trustworthy-connection-state`, after `confirm-backend-ready-before-running`, and rebases onto its edits in `crates/process/src/manager.rs`, `crates/ui/src/connection.rs` and `crates/ui/src/app.rs`. It introduces the shared `StatusView` struct that `detect-dead-proxy-link` later extends.


### `l1` — tasks 1.1, 1.2 — seam `exit-records`

- order: parallel, shard `process`; coder `rust-coder`; packages `v2ray-rs-process`
- sites:
  - `crates/process/src/manager.rs` · `StopReason (new enum) + stop_reason field on ProcessManager` · anchor `const REASON_MAX_CHARS: usize = 200;` — add `pub enum StopReason { UserStop, NodeSwitch, ApplyRestart, AppQuit, StartFailed }` with a `as_str()`/Display yielding user-stop|node-switch|apply-restart|app-quit|start-failed; store the pending reason on the manager (default UserStop)
  - `crates/process/src/manager.rs` · `ProcessManager::new` · anchor `cached_version: None,` — initialize the new stop_reason field alongside the existing ones
  - `crates/process/src/manager.rs` · `ProcessManager::shutdown` · anchor `pub async fn shutdown(&mut self) {` — add `shutdown_with(&mut self, reason: StopReason)` that sets the stored reason then runs the current body; keep `shutdown()` delegating with UserStop
  - `crates/process/src/manager.rs` · `ProcessManager::stop` · anchor `pub async fn stop(&mut self) -> Result<(), ProcessError> {` — stop() keeps the reason already stored by shutdown_with; no signature change unless a stop_with is needed for the Error-without-child path
  - `crates/process/src/manager.rs` · `launch (TUN device timeout)` · anchor `return Err(ProcessError::TunDeviceTimeout(rt.iface.clone()));` — set stop_reason = StartFailed before the preceding graceful_stop() so the exit record reads reason=start-failed
  - `crates/process/src/manager.rs` · `launch (xray-up failure)` · anchor `return Err(ProcessError::TunHelper(e));` — same: StartFailed before the graceful_stop() on helper failure
  - `crates/process/src/manager.rs` · `write_exit_record` · anchor `fn write_exit_record(&self, requested: bool, status: Option<&ExitStatus>) {` — print `reason=` after `requested=`: the stored StopReason for requested stops, literal `crash` for requested=false; keep requested/crashes_in_window/last_output
  - `crates/process/src/manager.rs` · `graceful_stop` · anchor `self.write_exit_record(true, status.as_ref());` — pass the stored reason through (or read it inside write_exit_record)
  - `crates/process/src/manager.rs` · `handle_unexpected_exit / wait_and_handle_exit crash records` · anchor `self.write_exit_record(false, Some(&status));` — unrequested path prints reason=crash; the Wait-error record at `self.write_exit_record(false, None);` likewise
  - `crates/process/src/lib.rs` · `crate exports` · anchor `pub use manager::{ProcessError, ProcessManager};` — re-export StopReason so crates/ui can name it
  - `crates/process/src/manager.rs` · `tests::exit_record_marks_requested_stop / exit_record_marks_unrequested_crash` · anchor `async fn exit_record_marks_requested_stop() {` — extend with per-reason assertions; add stub tests for TUN device timeout → reason=start-failed and crash → requested=false reason=crash
  - `crates/process/src/manager.rs` · `last_error_line (new, beside last_output_line)` · anchor `fn last_output_line(&self) -> Option<String> {` — add `fn last_error_line(&self) -> Option<String>` scanning the same last_n(50) for [Warning]/[Error]/WARN/ERROR/FATAL/panic tolerating ANSI; last_output_line unchanged
  - `crates/process/src/manager.rs` · `write_exit_record` · anchor `"requested={requested} {} crashes_in_window={} last_output={last}",` — append ` last_error=` with last_error_line() or `none`
  - `crates/process/src/manager.rs` · `tests (new: last error survives access noise)` · anchor `async fn exit_record_marks_unrequested_crash() {` — add a stub that prints a [Warning] then access lines → last_error is the warning, last_output the access line; assert the crash Error text is unchanged
  - `crates/process/src/manager.rs` · `readiness-failure exit path (added by confirm-backend-ready-before-running)` · anchor `self.write_exit_record(true, status.as_ref());` — After the rebase onto confirm-backend-ready-before-running there are THREE callers, not two: graceful_stop (requested=true, self.stop_reason.as_str()), handle_unexpected_exit and wait_and_handle_exit (requested=false, CRASH_REASON), and the readiness path (requested=false, StopReason::StartFailed.as_str(), crashes_in_window=0). The readiness path writes its own record and must not route through graceful_stop, which would emit requested=true reason=user-stop.
- work, in order:
  - TEST FIRST: add `exit_before_ready_records_start_failed` — FATAL-then-exit stub with a ready probe (the harness confirm-backend-ready-before-running adds), assert the exit record carries `requested=false reason=start-failed crashes_in_window=0` and that no `reason=user-stop` record is written
  - TEST FIRST: extend `exit_record_marks_requested_stop` to assert `reason=user-stop`, and add `exit_record_reason_per_stop_reason` driving shutdown_with for NodeSwitch/ApplyRestart/AppQuit and asserting `reason=node-switch|apply-restart|app-quit`
  - TEST FIRST: add `exit_record_marks_start_failed_on_tun_device_timeout` using `stub_helper` with an xray-up that exits 1 (ProcessError::TunHelper — instant). Do NOT use `xray_on_lo`: its iface is `lo`, which always exists, so wait_for_device returns immediately and the timeout never fires; the nonexistent-iface variant costs a fixed tun::DEVICE_TIMEOUT of 10 s that no manager field overrides. Assert `requested=true reason=start-failed`
  - TEST FIRST: extend `exit_record_marks_unrequested_crash` to assert `requested=false reason=crash`
  - TEST FIRST: add `last_error_survives_access_noise` — stub prints `[Warning] cert about to expire`, then two access-shaped stdout lines, then exits; assert `last_error=[Warning] cert about to expire` and `last_output=<final access line>`, and that the ProcessState::Error text still ends with the final buffered line (unchanged behavior)
  - TEST FIRST: add `last_error_is_none_without_warnings` and `last_error_matches_through_ansi` (stub prints `\033[33m[Warning]\033[0m tls retry`)
  - Add `pub enum StopReason { UserStop, NodeSwitch, ApplyRestart, AppQuit, StartFailed }` (Debug, Clone, Copy, PartialEq, Eq) with `pub fn as_str(&self) -> &'static str` yielding user-stop|node-switch|apply-restart|app-quit|start-failed and `impl std::fmt::Display` delegating to as_str, beside `const REASON_MAX_CHARS: usize = 200;` in crates/process/src/manager.rs; add `const CRASH_REASON: &str = "crash";` there too
  - Add field `stop_reason: StopReason` to `ProcessManager`, initialized to `StopReason::UserStop` next to `cached_version: None,` in `ProcessManager::new`
  - Add `pub async fn shutdown_with(&mut self, reason: StopReason)` holding today's `shutdown` body preceded by `self.stop_reason = reason;`; `shutdown` becomes `self.shutdown_with(StopReason::UserStop).await`
  - Set `self.stop_reason = StopReason::StartFailed;` immediately before the `graceful_stop()` calls guarding `ProcessError::TunDeviceTimeout` and `ProcessError::TunHelper` in `launch`
  - Change the signature to `fn write_exit_record(&self, requested: bool, reason: &str, status: Option<&ExitStatus>)` and emit `requested={requested} reason={reason} {status} crashes_in_window={} last_output={last} last_error={err}`; graceful_stop passes `self.stop_reason.as_str()`, both unrequested sites pass `CRASH_REASON`
  - Add `fn last_error_line(&self) -> Option<String>` beside `last_output_line`, scanning `buffer.last_n(50)` in reverse for a line whose ANSI-stripped content contains any of `[Warning]`, `[Error]`, `WARN`, `ERROR`, `FATAL`, `panic`, returning it through `truncate_reason`; add the private helper `fn without_ansi(line: &str) -> String` next to `truncate_reason`
  - Re-export `StopReason` from crates/process/src/lib.rs at anchor 'pub use manager::{ProcessError, ProcessManager};'
  - Emit the readiness-failure exit record from the readiness path itself with `write_exit_record(false, StopReason::StartFailed.as_str(), Some(&status))`; do NOT let it fall through to graceful_stop, whose stored `stop_reason` defaults to UserStop and would emit `requested=true reason=user-stop` — banned by this seam's own forbidden list
  - Reset `self.stop_reason = StopReason::UserStop;` after `write_exit_record` so a launch failure's StartFailed does not leak into a later stop on the same manager
- verify: `make test-process && make lint`

### `l2` — tasks 1.3 — seam `session-fields`

- order: after `l1` (shared `crates/process/src`); coder `rust-coder`; packages `v2ray-rs-core`, `v2ray-rs-process`, `v2ray-rs-ui`
- sites:
  - `crates/process/src/manager.rs` · `ProcessManager::with_log_file / with_session_fields` · anchor `pub fn with_log_file(mut self, writer: Option<Arc<RotatingFileWriter>>) -> Self {` — add builder `with_session_fields(mut self, fields: String) -> Self` storing an Option<String>
  - `crates/process/src/manager.rs` · `write_session_record` · anchor `&format!("backend={backend} version={version} node={node} tun={tun}"),` — append the caller-supplied session fields (truncate_reason-sanitized) after tun=
  - `crates/ui/src/connection.rs` · `candidate loop manager build` · anchor `let mut mgr = configure(` — chain .with_session_fields(format!("hijack={} capture_dns={} strict={} nodes_pinned={} profile={}", ...)) from effective_settings.tun.dns_hijack, the built TunRuntime's capture_dns/strict, `pinned`, and whether resolve_effective_config used an imported profile
  - `crates/ui/src/connection.rs` · `resolve_effective_config call (profile override detection)` · anchor `let (mut effective_rules, mut effective_settings) = resolve_effective_config(` — capture whether the candidate's routing/DNS came from an imported profile → the `profile=imported|app` field (resolve_effective_config currently returns no such flag — see risks)
  - `crates/ui/src/connection.rs` · `tests::live_connect_writes_backend_diagnostics` · anchor `contents.contains("backend=sing-box version=1.13.0 node=203.0.113.1 tun=off"),` — existing assertion must absorb the new suffix; add an xray TUN stub test asserting the session line's hijack/capture_dns/strict/nodes_pinned/profile fields
  - `crates/core/src/models/imported_profile.rs` · `uses_imported_profile (new)` · anchor `pub fn resolve_effective_config(` — Add `pub fn uses_imported_profile(node_ref: &ConnectionNodeRef, subscriptions: &[Subscription]) -> bool` holding the exact condition of this function's first arm, and make that arm read through it so the predicate and the resolution cannot drift.
  - `crates/core/src/models/mod.rs` · `imported_profile re-export` · anchor `pub use imported_profile::{ImportedProfile, resolve_effective_config};` — Extend the re-export with `uses_imported_profile`.
- work, in order:
  - TEST FIRST (core): add `uses_imported_profile_matches_resolve_effective_config` in crates/core/src/models/imported_profile.rs — true for a subscription node whose subscription has use_imported_profile + imported_profile, false for a manual node, false when use_imported_profile is off
  - TEST FIRST (process): add `session_record_appends_caller_fields` — `with_session_fields("hijack=hijack capture_dns=true strict=false nodes_pinned=true profile=app".into())`, assert the single session line contains `tun=off hijack=hijack` and the whole suffix; add `session_fields_cannot_forge_a_second_line` with an embedded \n
  - TEST FIRST (ui): update `live_connect_writes_backend_diagnostics` so the existing assertion absorbs the suffix (assert `contents.contains("backend=sing-box version=1.13.0 node=203.0.113.1 tun=off")` still holds and the same line ends with `hijack=off capture_dns=false strict=false nodes_pinned=true profile=app`)
  - TEST FIRST (ui): add `xray_tun_session_record_carries_dns_decisions` — stub xray backend, `tun_settings()` with `capless_probe`, assert the session line contains `tun=on hijack=hijack capture_dns=true strict=false nodes_pinned=true profile=app`
  - Add `pub fn uses_imported_profile(node_ref: &ConnectionNodeRef, subscriptions: &[Subscription]) -> bool` in crates/core/src/models/imported_profile.rs holding the exact condition of resolve_effective_config's first arm; make resolve_effective_config's arm read through it so the two cannot drift; extend the re-export at anchor 'pub use imported_profile::{ImportedProfile, resolve_effective_config};'
  - Add field `session_fields: Option<String>` to `ProcessManager` and builder `pub fn with_session_fields(mut self, fields: String) -> Self` following the with_log_file shape
  - In `write_session_record`, append `truncate_reason(&fields)` after `tun={tun}` when session_fields is Some
  - In crates/ui/src/connection.rs add `fn hijack_field(mode: DnsHijackMode) -> &'static str` (hijack|native|disabled) and chain `.with_session_fields(format!("hijack={} capture_dns={} strict={} nodes_pinned={} profile={}", ..))` onto the builder at anchor '            let mut mgr = configure(' — values from the built `tun` runtime (None → `off`/`false`/`false`), `pinned`, and `uses_imported_profile(&candidate.node_ref, &subscriptions)` → `imported`|`app`
- verify: `make test-core && make test-process && make test-ui && make lint`

### `l3` — tasks 2.1 — seam `stop-reason-plumbing`

- order: after `l2` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-process`, `v2ray-rs-ui`
- sites:
  - `crates/ui/src/connection.rs` · `ConnectionCmd` · anchor `enum ConnectionCmd {` — `Stop(StopReason)`
  - `crates/ui/src/connection.rs` · `ConnectionHandle::stop` · anchor `let _ = self.cmd_tx.try_send(ConnectionCmd::Stop);` — take a StopReason parameter and send it
  - `crates/ui/src/connection.rs` · `queued-stop check before start` · anchor `// A Stop that arrived while we were queued behind the previous` — match `Ok(ConnectionCmd::Stop(_))`; nothing started yet so no shutdown_with
  - `crates/ui/src/connection.rs` · `candidate-loop head stop check` · anchor `'candidates: for candidate in candidates {` — match Stop(reason) and pass it to parked.shutdown_with(reason)
  - `crates/ui/src/connection.rs` · `select! stop during start_with_connection` · anchor `let started = tokio::select! {` — Stop(reason) → mgr.shutdown_with(reason) and parked.shutdown_with(reason)
  - `crates/ui/src/connection.rs` · `stop queued behind a successful start` · anchor `// A Disconnect clicked while the start was in flight sits` — match Stop(reason) → mgr.shutdown_with(reason)
  - `crates/ui/src/connection.rs` · `supervision loop stop arm` · anchor `Some(ConnectionCmd::Stop) = cmd_rx.recv() => {` — the arm inside the `loop { tokio::select! {` that follows the comment `// The manager restarts in place on an unexpected exit; if` — Stop(reason) → mgr.shutdown_with(reason)
  - `crates/ui/src/connection.rs` · `host-level failure teardown` · anchor `if grant_fixable(&e) {` — the mgr.shutdown()/parked.shutdown() above it become shutdown_with(StopReason::StartFailed)
  - `crates/ui/src/app.rs` · `stop_reason_for (new pure fn) + AppMsg::Disconnect arm` · anchor `AppMsg::Disconnect => {` — map reconnect_pending→ApplyRestart, pending_direct_target→NodeSwitch, pending_exit→AppQuit, else UserStop; pass to handle.stop(reason) at `handle.stop();` inside DisconnectPlan::Stop
  - `crates/ui/src/app.rs` · `App::quit` · anchor `fn quit(&mut self, sender: &ComponentSender<Self>) {` — QuitPlan::Stop calls handle.stop(StopReason::AppQuit)
  - `crates/ui/src/app.rs` · `AppMsg::ApplyAndRestart` · anchor `AppMsg::ApplyAndRestart => {` — unchanged flag-setting; it is the ApplyRestart input to the mapping (reconnect_pending = true before Disconnect)
  - `crates/ui/src/app.rs` · `AppMsg::ConnectToNode node-switch path` · anchor `self.pending_direct_target = Some(target);` — the NodeSwitch input to the mapping; no behavior change beyond the reason
  - `crates/ui/src/app.rs` · `tests (new mapping unit test)` · anchor `fn reconnect_pending_consumed_on_stop_or_error() {` — add a table test for stop_reason_for over (reconnect_pending, pending_direct_target, pending_exit) next to the existing pure-fn tests
- work, in order:
  - TEST FIRST (app.rs): add `stop_reason_for_maps_every_input` — a table over (pending_exit, reconnect_pending, direct_target.is_some()) asserting AppQuit / ApplyRestart / NodeSwitch / UserStop with the precedence above
  - TEST FIRST (connection.rs): add `stop_reason_reaches_the_exit_record` — connect a stub, `handle.stop(StopReason::NodeSwitch)`, wait for Stopped, assert backend.log's exit record contains `reason=node-switch`
  - Change `enum ConnectionCmd { Stop }` to `Stop(StopReason)` and `ConnectionHandle::stop(&self, reason: StopReason)` at anchor '        let _ = self.cmd_tx.try_send(ConnectionCmd::Stop);'
  - Update all six Stop match sites per the transition table, passing the reason to `shutdown_with`; the host-level teardown at anchor '                        if grant_fixable(&e) {' uses `StopReason::StartFailed`
  - Add `fn stop_reason_for(pending_exit: bool, reconnect_pending: bool, direct_target: bool) -> StopReason` in app.rs beside `disconnect_plan`, and call it in the DisconnectPlan::Stop arm at anchor '                            handle.stop();'
  - `App::quit`'s QuitPlan::Stop arm calls `handle.stop(StopReason::AppQuit)`
  - Import `StopReason` in crates/ui/src/app.rs and connection.rs from `v2ray_rs_process`
- verify: `make test-ui && make lint`

### `l4` — tasks 2.3 — seam `backend-state-logs`

- order: after `l3` (shared `crates/process/src`); coder `rust-coder`; packages `v2ray-rs-process`
- sites:
  - `crates/process/src/state.rs` · `StateManager::transition` · anchor `let old = self.state.transition(target.clone())?;` — log::info! `backend state {old:?} → {target:?}` after a successful transition
  - `crates/process/src/manager.rs` · `handle_unexpected_exit` · anchor `async fn handle_unexpected_exit(&mut self, status: ExitStatus) {` — after record_crash/write_exit_record log `backend exited unexpectedly (code=…); respawn crash=i/3`
- work, in order:
  - TEST FIRST (state.rs): add `rejected_transition_changes_nothing` asserting an invalid transition returns Err and leaves `mgr.state()` unchanged (the observable guard behind 'no line for a rejected transition')
  - TEST FIRST (manager.rs): add `crash_respawn_records_crash_count` asserting the exit records from a `crashing_backend(dir.path(), 1)` run carry `crashes_in_window=1` then a second session record follows — the same ordering the respawn line describes
  - Log `log::info!("backend state {old:?} → {target:?}")` in `StateManager::transition` after the inner transition succeeds
  - Log `log::info!("backend exited unexpectedly ({}); respawn crash={}/{}", exit_status_field(Some(&status)), self.crash_times.len(), MAX_CRASHES)` in `handle_unexpected_exit` INSIDE the restart branch — after both the `!self.auto_restart` guard and the `crash_times.len() >= MAX_CRASHES` guard, so a give-up never prints a respawn claim, and keep the give-up branches free of a respawn claim
- verify: `make test-process && make lint`

### `l5` — tasks 2.2 — seam `decision-logs`

- order: after `l4` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/app.rs` · `AppMsg::Connect` · anchor `AppMsg::Connect(origin) => {` — log::info! `connect origin=… strategy=… candidates=n profile_override=bool` after the candidate list is planned
  - `crates/ui/src/app.rs` · `App::start_connection` · anchor `fn start_connection(` — alternative single choke point for the connect record (both Connect and ConnectToNode reach it); origin must be threaded in
  - `crates/ui/src/app.rs` · `App::schedule_auto_reconnect` · anchor `fn schedule_auto_reconnect(&mut self, sender: &ComponentSender<Self>) -> bool {` — log `auto-reconnect scheduled attempt=i/3 delay=5s` on success and `auto-reconnect exhausted` on the refused branch
  - `crates/ui/src/app.rs` · `App::quit` · anchor `let plan = quit_plan(` — log `quit requested plan=…` once the plan is known
  - `crates/ui/src/connection.rs` · `candidate start/failure records` · anchor `let candidate_address = candidate.node.address().to_string();` — log `candidate start i/n label=…` here; log `candidate failed i/n label=… reason=…` at each failures.push site (config-generation failure, start error, crash give-up) before the next candidate begins
- work, in order:
  - TEST FIRST (logging.rs): add `#[cfg(test)] pub(crate) struct TestLogCapture` over a `Mutex<Vec<String>>` implementing `Log`, `pub(crate) fn install_test_capture() -> &'static TestLogCapture` installing it once via `std::sync::Once` + `log::set_boxed_logger`, with `lines_containing(&self, needle: &str) -> Vec<String>` and `wait_for(&self, needle: &str, count: usize) -> Vec<String>` (5 s cap, 50 ms poll); unit-test the capture itself with two records
  - TEST FIRST (connection.rs): add `failover_is_traceable` — three candidates, first fails, use addresses used by no other test in the binary — 198.51.100.11/.12/.13 — and filter the capture on those; assert it holds `candidate start 1/3` with 198.51.100.11, then `candidate failed 1/3` carrying its reason, then `candidate start 2/3`, in that order
  - TEST FIRST (app.rs): add table tests for `connect_line`, `auto_reconnect_line`, `auto_reconnect_exhausted_line` and `quit_line` asserting the exact key=value text, including `candidates=3` and `attempt=1/3 delay=5s`
  - Add the pure formatters in app.rs beside `quit_plan`: `fn connect_line(origin: ConnectOrigin, strategy: AutoResolveStrategy, candidates: usize, profile_override: bool) -> String`, `fn auto_reconnect_line(attempt: u32, max: u32) -> String`, `fn quit_line(plan: &QuitPlan) -> String`, and an `origin_field(ConnectOrigin) -> &'static str` mapping User→user (node when a direct target is set), AutoReconnect→auto-reconnect, Restart→restart
  - Log the connect record in `start_connection` after the guards pass, using the `origin` parameter added in the status-text seam and `uses_imported_profile` over the candidate list for `profile_override`
  - Log `auto-reconnect scheduled` / `auto-reconnect exhausted` in `schedule_auto_reconnect`, and `quit requested plan=…` in `App::quit` after `quit_plan` returns
  - In connection.rs log `candidate start i/n label=…` at anchor '            let candidate_address = candidate.node.address().to_string();' and `candidate failed i/n label=… reason=…` at each `failures.push(CandidateFailure::new(` site, reusing the stripped reason
- verify: `make test-ui && make lint`

### `l6` — tasks 3.1, 3.2 — seam `unclean-exit-paths`

- order: after `l5` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/app.rs` · `App::init (signal handlers)` · anchor `let show_wizard = settings_load_error.is_none() && !settings.onboarding_complete;` — install glib::unix_signal_add_local for SIGTERM/SIGINT/SIGHUP near the existing background-task block; each logs the signal and emits the quit path (AppMsg::TrayQuit); a second signal while pending_exit exits immediately
  - `crates/ui/src/app.rs` · `AppMsg::TrayQuit` · anchor `AppMsg::TrayQuit => {` — the quit path the signal handler reuses; pending_exit gate for the force-exit on a second signal lives in App::quit
  - `crates/ui/src/app.rs` · `try_run (panic hook)` · anchor `crate::logging::init_logging(&paths);` — install a panic hook right after, chaining std::panic::take_hook, logging error with payload message + location
  - `crates/ui/src/logging.rs` · `install_panic_hook + test` · anchor `pub fn init_logging(paths: &AppPaths) {` — the hook and its unit test belong beside AppLogger, which already has a direct-construction test harness (log_record)
  - `crates/ui/src/app.rs` · `App::quit second-signal guard` · anchor `fn quit(` — Add `fn force_exit_on_second_signal(pending_exit: bool) -> bool` and an early return in App::quit — today there is no pending_exit early return, so with the handle already taken quit_plan returns AwaitStopped and a second signal keeps waiting.
- work, in order:
  - TEST FIRST (logging.rs): add `panic_record_names_message_and_location` over the pure `panic_record`, and `install_panic_hook_chains_previous` asserting a sentinel set by a previously installed hook still fires
  - Add `fn panic_record(info: &std::panic::PanicHookInfo) -> String` and `pub fn install_panic_hook()` (chaining `std::panic::take_hook`) to crates/ui/src/logging.rs; call `install_panic_hook()` right after `crate::logging::init_logging(&paths);`
  - Install `glib::unix_signal_add_local` handlers for SIGTERM (15), SIGINT (2) and SIGHUP (1) in `App::init`, each logging the signal and emitting `AppMsg::TrayQuit`, returning `glib::ControlFlow::Continue`; store the SourceIds on the model; the second-signal force exit lives in `App::quit` behind the existing `pending_exit` flag
  - TEST FIRST: add `force_exit_on_second_signal` table test — false on the first signal (pending_exit false), true on the second (pending_exit true)
  - Add the pure `fn force_exit_on_second_signal(pending_exit: bool) -> bool` beside `quit_plan` and take the early exit path in App::quit when it returns true
- verify: `make test-ui && make lint`

### `l7` — tasks 3.3 — seam `unclean-exit-paths`

- order: after `l6` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-process`, `v2ray-rs-ui`
- sites:
  - `crates/ui/src/app.rs` · `startup orphan reap` · anchor `if !skip_orphans && let Err(err) = cleanup_orphaned_backend(&orphan_paths) {` — before the reap, if paths.pid_file_path() exists, append `unclean previous run left backend pid=… killed=bool` to logs_dir()/backend.log through a short-lived RotatingFileWriter
  - `crates/ui/src/app.rs` · `cleanup_orphaned_backend` · anchor `fn cleanup_orphaned_backend(paths: &AppPaths) -> std::io::Result<bool> {` — read the PID via PidFile::read() before check_and_kill_orphaned so the record carries the pid; returns whether a live orphan was killed
  - `crates/ui/src/app.rs` · `tests (new stale-PID test)` · anchor `fn quit_idle_exits() {` — add a temp-profile test writing a stale PID ownership record then asserting backend.log gains the unclean-previous-run record (needs AppPaths::for_profile_in like connection.rs's stub())
- work, in order:
  - TEST FIRST (app.rs): add `stale_pid_file_records_unclean_previous_run` — temp profile, stale PID ownership record, call `record_unclean_previous_run(&paths)`, assert backend.log contains `unclean previous run left backend pid=` and the pid; add `no_pid_file_records_nothing`
  - Extract `fn record_unclean_previous_run(paths: &AppPaths) -> bool` in app.rs: read the PID via `PidFile::read()` before the reap, run `cleanup_orphaned_backend`, and append `unclean previous run left backend pid=… killed=…` through a short-lived `RotatingFileWriter::open(paths.logs_dir().join("backend.log"), DEFAULT_MAX_BYTES)`; call it in the startup block in place of the bare `cleanup_orphaned_backend` call
- verify: `make test-process && make test-ui && make lint`

### `l8` — tasks 4.1 — seam `status-text`

- order: after `l7` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/app.rs` · `status_primary (new pure fn) + update_status_labels` · anchor `(ProcessState::Starting, _) => ("Connecting…".to_string(), "Resolving nodes".into()),` — replace the Starting arm with status_primary(state, prev_state, origin, attempt) → "Restarting after crash" / "Reconnecting (n/3)" / "Connecting…"
  - `crates/ui/src/app.rs` · `App::apply_state` · anchor `let from = self.process_state.clone();` — the previous state needed by status_primary is already captured here; store it (or pass it) for update_status_labels
  - `crates/ui/src/app.rs` · `App struct fields` · anchor `auto_reconnect_attempts: u32,` — add the current connection's ConnectOrigin (and reuse auto_reconnect_attempts as the attempt number); set it where start_connection is called
  - `crates/ui/src/app.rs` · `tests (status_primary table)` · anchor `fn toggle_disabled_while_stopping() {` — add unit tests for each status_primary case beside the existing pure-fn tests
- work, in order:
  - TEST FIRST: add `status_texts_covers_every_starting_case` — a table over StatusView asserting Restarting after crash / Reconnecting (1/3) / Connecting… and that the Running, Stopping, Stopped and Error arms are byte-identical to today's strings
  - Add `struct StatusView { state: ProcessState, prev_state: ProcessState, meta: Option<ConnectionMetadata>, origin: ConnectOrigin, attempt: u32 }` and `fn status_texts(view: &StatusView) -> (String, String)` in app.rs, holding today's `update_status_labels` match plus the two new Starting arms (the sibling change adds `health` and `dns_failing` fields and the Running arms that read them)
  - Add fields `status_prev: ProcessState` and `connection_origin: ConnectOrigin` to `App`; set `status_prev` from the `from` already captured in `apply_state`, and force it to `ProcessState::Stopped` in `start_connection` before its `apply_state(&ProcessState::Starting)`
  - Add an `origin: ConnectOrigin` parameter to `fn start_connection(` and thread it from both call sites (the AppMsg::Connect arm and the AppMsg::ConnectToNode arm), storing it in `connection_origin`
  - Rewrite `update_status_labels` to build a `StatusView` from the model and call `status_texts`
- verify: `make test-ui && make lint`

### `l9` — tasks 5.1, 5.2 — seam `verification`

- order: after `l8` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-core`, `v2ray-rs-process`, `v2ray-rs-ui`
- work, in order:
  - Run the full floor and paste the failing output verbatim if anything is red
  - MANUAL: execute the task 5.2 live pass and record the observed exit reasons and app-log lines
- verify: `TEST_TIMEOUT=10m make test && make lint`

### Contracts


**`exit-records`** — tasks 1.1, 1.2
- states: `stop_reason=UserStop`, `stop_reason=NodeSwitch`, `stop_reason=ApplyRestart`, `stop_reason=AppQuit`, `stop_reason=StartFailed`, `exit_record_requested`, `exit_record_crash`, `exit_record_absent`, `last_error=line`, `last_error=none`, `error_text_unchanged`, `exit_record_start_failed`
- transitions:
  - `ProcessManager::new(..) constructed` → `stop_reason=UserStop` → **set** — anchor '            cached_version: None,' in ProcessManager::new — the new `stop_reason` field is initialized beside it
  - `shutdown() with no explicit reason` → `stop_reason=UserStop` → **set** — anchor '    pub async fn shutdown(&mut self) {' — shutdown() delegates to shutdown_with(StopReason::UserStop)
  - `shutdown_with(StopReason::NodeSwitch)` → `stop_reason=NodeSwitch` → **set** — anchor '    pub async fn shutdown(&mut self) {' — shutdown_with stores the reason before running today's body
  - `shutdown_with(StopReason::ApplyRestart)` → `stop_reason=ApplyRestart` → **set** — anchor '    pub async fn shutdown(&mut self) {'
  - `shutdown_with(StopReason::AppQuit)` → `stop_reason=AppQuit` → **set** — anchor '    pub async fn shutdown(&mut self) {'
  - `stop() called without a preceding shutdown_with` → `stop_reason=UserStop` → **no-op** — anchor '    pub async fn stop(&mut self) -> Result<(), ProcessError> {' — stop takes no reason parameter and leaves the stored reason alone
  - `TUN device did not appear within tun::DEVICE_TIMEOUT during launch` → `stop_reason=StartFailed` → **forced** — anchor '                return Err(ProcessError::TunDeviceTimeout(rt.iface.clone()));' — set before the graceful_stop() that precedes it
  - `xray-up helper returned Err during launch` → `stop_reason=StartFailed` → **forced** — anchor '                return Err(ProcessError::TunHelper(e));' — set before the preceding graceful_stop()
  - `graceful_stop completes (requested stop)` → `exit_record_requested` → **set** — anchor '        self.write_exit_record(true, status.as_ref());' at the tail of graceful_stop — prints `requested=true reason=<stop_reason.as_str()>`
  - `child exited while state was Running` → `exit_record_crash` → **forced** — anchor '        self.write_exit_record(false, Some(&status));' in handle_unexpected_exit — prints `requested=false reason=crash`
  - `child.wait() returned Err` → `exit_record_crash` → **forced** — anchor '                self.write_exit_record(false, None);' in wait_and_handle_exit — prints `requested=false reason=crash code=none`
  - `log_writer is None` → `exit_record_absent` → **no-op** — anchor '    fn write_exit_record(&self, requested: bool, status: Option<&ExitStatus>) {' — its first statement returns when log_writer.is_none()
  - `buffered output holds a [Warning] line followed by access lines` → `last_error=line` → **set** — anchor '    fn last_output_line(&self) -> Option<String> {' — last_error_line scans the same buffer.last_n(50); last_output stays the access line
  - `no matching warning/error token in the last 50 lines` → `last_error=none` → **set** — requirement sentence: 'the last warning, error, or fatal line among the recent output (or `none`)'
  - `matching line wrapped in CSI colour codes` → `last_error=line` → **set** — tasks.md 1.2: '(`[Warning]`, `[Error]`, `WARN`, `ERROR`, `FATAL`, `panic`, ANSI-tolerant)'
  - `crash exit with a last stderr line` → `error_text_unchanged` → **no-op** — anchor '        if let Some(reason) = self.last_output_line() {' in handle_unexpected_exit — last_output_line is not modified
  - `child exited before ready (readiness path)` → `exit_record_start_failed` → **forced** — tasks.md 1.1: 'a backend that exits before ready records requested=false reason=start-failed crashes_in_window=0 from the readiness path, never through graceful_stop'
- forbidden:
  - `reason=crash` together with `requested=true` — a requested stop is never a crash
  - `reason=user-stop` on any launch-failure path (TunDeviceTimeout, TunHelper)
  - write_exit_record accepting a `StopReason` for the unrequested path — `crash` is not a StopReason variant and must come from the single `CRASH_REASON` constant
  - changing `last_output_line`'s selection rule or the crash `Error` message text
  - more than one exit record per child exit
- seeding:
  - requested stop with a given reason: `mgr.shutdown_with(StopReason::NodeSwitch).await` (or the matching variant) after a successful `mgr.start().await`
  - crash record: `manager_for(&dir, "...exit 3")` + `mgr.set_auto_restart(false)` + `mgr.wait_and_handle_exit().await`, exactly as `exit_record_marks_unrequested_crash` does
  - last_error: stub body prints a `[Warning] ...` line, then access-shaped stdout lines, then exits — read the record via `wait_for_lines(&dir.path().join("backend.log"), 2)`
  - records are read from the file with `read_lines` / `wait_for_lines`, never from the broadcast channel
  - Seed `exit_record_marks_start_failed_on_tun_device_timeout` with a stub helper whose `xray-up` exits 1 (ProcessError::TunHelper), which is instant. Do NOT seed it with `xray_on_lo`: its iface is `lo`, which always exists, so wait_for_device returns immediately and the timeout never fires. The nonexistent-iface variant works but costs a fixed tun::DEVICE_TIMEOUT of 10 s that no manager field overrides.
- budgets:
  - last_error_line scans exactly the last 50 buffered lines (`buffer.last_n(50)`), the same window as last_output_line
  - reason and last_error text pass through `truncate_reason`: REASON_MAX_CHARS = 200 chars, newline-scrubbed
  - `wait_for_lines` polls up to 2 s for the expected line count
  - per-test wall clock: the seam's verify command caps the whole crate at 5 m
  - tun::DEVICE_TIMEOUT = 10 s, fixed and not per-manager overridable — the reason the helper-failure seeding is preferred

**`session-fields`** — tasks 1.3
- states: `session_fields=None`, `session_fields=Some(text)`, `session_record_single_line`, `session_record_repeated`, `tun_runtime=None`, `capture_dns=true`, `strict=false`, `profile=imported`, `profile=app`
- transitions:
  - `manager built without with_session_fields` → `session_fields=None` → **no-op** — anchor '            &format!("backend={backend} version={version} node={node} tun={tun}"),' in write_session_record — the line ends at tun=
  - `with_session_fields(text) chained in the builder` → `session_fields=Some(text)` → **set** — anchor '    pub fn with_log_file(mut self, writer: Option<Arc<RotatingFileWriter>>) -> Self {' — the builder shape with_session_fields copies
  - `field text containing \n or \r` → `session_record_single_line` → **forced** — anchor '    fn truncate_reason(line: &str) -> String {' comment 'Records are single-line'
  - `respawn after a crash` → `session_record_repeated` → **set** — anchor '        self.write_session_record().await;' — the first statement of launch, which respawn also calls
  - `TUN disabled or backend v2ray` → `tun_runtime=None` → **forced** — anchor '    if !settings.tun.enabled {' in build_tun_runtime — fields read hijack=off capture_dns=false strict=false
  - `xray + tun.enabled + dns_hijack Hijack + every node hostname pinned` → `capture_dns=true` → **set** — anchor '        capture_dns: backend == BackendType::Xray' in build_tun_runtime
  - `drop_strict_route decided for the connection` → `strict=false` → **forced** — anchor '            if drop_strict_route {' inside the candidate loop — effective_settings.tun.strict_route is cleared before build_tun_runtime
  - `candidate's subscription has use_imported_profile and an imported_profile` → `profile=imported` → **set** — anchor '        && sub.use_imported_profile' in resolve_effective_config
  - `manual node, or subscription without an imported profile` → `profile=app` → **set** — anchor '    (global_rules.to_vec(), settings.clone())' — the fallback arm of resolve_effective_config
- forbidden:
  - computing `profile=` by re-deriving the imported-profile condition inline in connection.rs — it must call the single core predicate
  - reading `settings.tun.*` rather than `effective_settings.tun.*` for the session fields (the imported profile and the strict-route drop change them)
  - the session fields naming a node's credentials, address, or port — only the five keys
  - more than one session record per launch
- seeding:
  - session_fields=Some: build through the production builder chain in connection.rs, or in process tests `manager_for(..).with_log_file(Some(backend_log(dir.path()))).with_session_fields("hijack=hijack capture_dns=true".into())`
  - tun_runtime=Some(rt) in a ui test: `tun_settings()` (or `strict_route_settings()`) passed to `connect(&stub, ..)` with the `capless_probe` configure hook
  - profile=imported: a `Subscription` fixture with `use_imported_profile = true` and `imported_profile = Some(..)`, reached through `resolve_effective_config` — never by setting the field on the record
  - read the session line from `stub.paths.logs_dir().join("backend.log")` after the first Running state, as `live_connect_writes_backend_diagnostics` does
- budgets:
  - exactly 1 session record per launch (`contents.matches(" session ").count() == 1` for a single-candidate connect)
  - session field text truncated at REASON_MAX_CHARS = 200
  - ui connect tests wait for a state at most 20 s (`next_state`)

**`stop-reason-plumbing`** — tasks 2.1
- states: `reason=UserStop`, `reason=NodeSwitch`, `reason=ApplyRestart`, `reason=AppQuit`, `reason=StartFailed`, `reason=carried`, `no_shutdown_call`, `terminal_state_unchanged`
- transitions:
  - `stop_reason_for(pending_exit=true, ..)` → `reason=AppQuit` → **forced** — anchor '    fn quit(&mut self, sender: &ComponentSender<Self>) {' — QuitPlan::Stop sets pending_exit then stops the handle
  - `stop_reason_for(false, reconnect_pending=true, direct_target=None)` → `reason=ApplyRestart` → **set** — anchor '                self.reconnect_pending = true;' in the AppMsg::ApplyAndRestart arm, dispatched straight into AppMsg::Disconnect
  - `stop_reason_for(false, false, direct_target=Some)` → `reason=NodeSwitch` → **set** — anchor '                    self.pending_direct_target = Some(target);' in the AppMsg::ConnectToNode arm, which sets reconnect_pending = false first
  - `stop_reason_for(false, false, None)` → `reason=UserStop` → **set** — anchor '            AppMsg::Disconnect => {' — the plain Disconnect click
  - `App::quit with QuitPlan::Stop` → `reason=AppQuit` → **forced** — anchor '                    handle.stop();' inside QuitPlan::Stop — becomes handle.stop(StopReason::AppQuit)
  - `Stop received while the task is still queued behind the previous teardown` → `no_shutdown_call` → **no-op** — anchor '        // A Stop that arrived while we were queued behind the previous' — nothing has started yet; Stopped is reported
  - `Stop received at the candidate-loop head` → `reason=carried` → **set** — anchor "'candidates: for candidate in candidates {" — passed to parked.shutdown_with(reason)
  - `Stop received during start_with_connection` → `reason=carried` → **set** — anchor '            let started = tokio::select! {' — passed to mgr.shutdown_with and parked.shutdown_with
  - `Stop queued behind a successful start` → `reason=carried` → **set** — anchor '                    // A Disconnect clicked while the start was in flight sits'
  - `Stop received in the supervision loop` → `reason=carried` → **set** — anchor '                    Some(ConnectionCmd::Stop) = cmd_rx.recv() => {'
  - `host-level start failure teardown` → `reason=StartFailed` → **forced** — anchor '                        if grant_fixable(&e) {' — the shutdown() calls above it
  - `any Stop path` → `terminal_state_unchanged` → **no-op** — anchor '    fn relays(state: &ProcessState) -> bool {' — terminal states stay with the supervising loop
- forbidden:
  - `reason=UserStop` reaching the manager on a host-level start failure or a queued-behind-start teardown
  - inferring the reason inside connection.rs from state rather than carrying it in the command
  - a `handle.stop()` call site left without an explicit reason (the parameter is mandatory, no Default)
  - changing which terminal state each Stop path reports
- seeding:
  - stop_reason_for is a free function: call it directly with the three booleans/Option in a table test, next to `reconnect_pending_consumed_on_stop_or_error`
  - connection-side reasons: drive `handle.stop(StopReason::NodeSwitch)` in a `connect(&stub, ..)` test and read `reason=node-switch` back from `stub.paths.logs_dir().join("backend.log")`
  - app.rs tests never construct an App or a GTK widget — the mapping must therefore stay a free function over plain inputs
- budgets:
  - exactly 6 `ConnectionCmd::Stop` match sites in connection.rs after the change (queued-before-start, loop head, select! during start, queued-after-start, supervision loop, plus the host-level teardown that constructs StartFailed itself)
  - cmd channel capacity stays 4 (`mpsc::channel::<ConnectionCmd>(4)`)
  - ui connect tests wait at most 20 s per state (`next_state`)

**`backend-state-logs`** — tasks 2.3
- states: `transition_accepted`, `transition_rejected`, `respawn_pending`, `respawn_given_up`, `exit_signal_reported`, `no_logger_installed`
- transitions:
  - `StateManager::transition with a valid target` → `transition_accepted` → **set** — anchor '        let old = self.state.transition(target.clone())?;' — one `backend state {old:?} → {target:?}` line at info after it succeeds
  - `StateManager::transition with an invalid target` → `transition_rejected` → **no-op** — anchor '            return Err(TransitionError::Invalid {' in ProcessState::transition — the `?` returns before the log
  - `unexpected exit with auto_restart on and crash budget left` → `respawn_pending` → **set** — anchor '    async fn handle_unexpected_exit(&mut self, status: ExitStatus) {' — one `backend exited unexpectedly (code=…); respawn crash=i/3` line after record_crash/write_exit_record
  - `unexpected exit with auto_restart off` → `respawn_given_up` → **forced** — anchor '        if !self.auto_restart {' in handle_unexpected_exit — the exit is recorded without a respawn claim
  - `crash_times.len() >= MAX_CRASHES` → `respawn_given_up` → **forced** — anchor '            if self.crash_times.len() >= MAX_CRASHES {'
  - `exit by signal (code None)` → `exit_signal_reported` → **set** — anchor '    fn exit_status_field(status: Option<&ExitStatus>) -> String {' — reused so the app log and the exit record agree
  - `state.rs unit tests transitioning directly` → `no_logger_installed` → **no-op** — codemap risk: 'StateManager::transition logging fires from tests too'
- forbidden:
  - logging a rejected transition
  - logging inside `ProcessState::transition` (the pure state machine) rather than `StateManager::transition`
  - a per-poll or per-log-line record — only state changes and respawn decisions
  - the respawn line disagreeing with the exit record about the code/signal
- seeding:
  - transition_accepted: `StateManager::new()` then `mgr.transition(ProcessState::Starting, None).unwrap()` — the shape `transition_returns_old_state` already uses
  - transition_rejected: `mgr.transition(ProcessState::Running, None)` from Stopped, asserting `is_err()`
  - respawn_pending: `crashing_backend(dir.path(), 1)` + `wait_for_lines`, as `failed_respawn_is_retried` does — never by calling handle_unexpected_exit directly
  - assert the recorded facts through the state broadcast (`drain_states`) and backend.log, not through the `log` façade: no capturing logger exists in this crate
- budgets:
  - MAX_CRASHES = 3 within CRASH_WINDOW = 60 s bounds the respawn lines per session
  - one log line per accepted transition — a normal connect emits Stopped→Starting and Starting→Running only
  - `wait_for_lines` polls up to 2 s

**`decision-logs`** — tasks 2.2
- states: `connect_recorded`, `connect_not_recorded`, `candidate_started`, `candidate_failed`, `failover_ended`, `reconnect_scheduled`, `reconnect_exhausted`, `quit_recorded`, `node_named_without_credentials`
- transitions:
  - `start_connection reached with a planned candidate list` → `connect_recorded` → **set** — anchor '    fn start_connection(' — `connect origin=… strategy=… candidates=n profile_override=…`, the single choke point both Connect and ConnectToNode reach
  - `start_connection refused by a guard (no binary, geodata missing, IPv6 gate)` → `connect_not_recorded` → **no-op** — anchor '                self.show_toast("No backend binary configured — check Preferences");' — the record is written only after the guards pass
  - `a candidate's manager is about to start` → `candidate_started` → **set** — anchor '            let candidate_address = candidate.node.address().to_string();' — `candidate start i/n label=…`
  - `config generation failed for a candidate` → `candidate_failed` → **set** — anchor '                            &format!("config generation failed: {e}"),' — same text as the CandidateFailure entry
  - `start_with_connection returned a non-host-level Err` → `candidate_failed` → **set** — anchor '    impl CandidateFailure { fn new(label: &str, reason: &str, address: &str, port: u16) -> Self {' — reason = strip_ansi(reason)
  - `the manager gave up after its crash budget` → `candidate_failed` → **set** — anchor '                            ProcessState::Error(msg) => {' inside the supervision loop — logged before the next candidate begins
  - `host-level failure` → `failover_ended` → **forced** — anchor '                    if e.is_host_level() {' — the loop returns, so no later candidate line follows
  - `schedule_auto_reconnect returns true` → `reconnect_scheduled` → **set** — anchor '        self.auto_reconnect_attempts += 1;' — `auto-reconnect scheduled attempt=i/3 delay=5s`
  - `auto_reconnect_allowed refuses (pending_exit or attempts == MAX_AUTO_RECONNECTS)` → `reconnect_exhausted` → **set** — anchor '        if !auto_reconnect_allowed(self.pending_exit, self.auto_reconnect_attempts) {'
  - `App::quit computes a plan` → `quit_recorded` → **set** — anchor '        let plan = quit_plan(' — `quit requested plan=…`
  - `any decision record` → `node_named_without_credentials` → **forced** — requirement sentence: 'Records SHALL identify nodes by name and SHALL NOT include credentials'
- forbidden:
  - a candidate-failed line whose reason differs from the text in `CandidateFailure`'s summary (both come from the one already-stripped string)
  - changing `terminal_failure(&failures)` or the `last_candidate_failure_reports_one_error` expected text
  - logging the node's password, uuid, or subscription URL
  - a connect line written for an attempt that never started
  - installing the capturing logger outside `#[cfg(test)]`, or installing it more than once per test binary
- seeding:
  - connect/auto-reconnect/quit lines: the app.rs formatter functions are free functions — unit-test them directly; app.rs tests never build an App
  - reconnect_exhausted: call `auto_reconnect_allowed(false, MAX_AUTO_RECONNECTS)` directly, as `auto_reconnect_exhausted_after_three_attempts` does
  - The capture is one process-wide logger per test binary: `candidate("203.0.113.1"/.2/.3)` appears in at least eight other connection.rs tests, all multi_thread and concurrent under --test-threads=4. A test asserting on capture ORDER must use addresses unique to itself (198.51.100.11/.12/.13) and filter on them.
- budgets:
  - MAX_AUTO_RECONNECTS = 3 and AUTO_RECONNECT_DELAY = 5 s are the numbers the reconnect line prints
  - one connect line per start_connection, one start line per candidate, at most one failed line per candidate
  - `TestLogCapture::wait_for(needle, count)` polls at most 5 s at 50 ms
  - log level: every decision line is `info`, i.e. visible at the default LevelFilter::Info

**`unclean-exit-paths`** — tasks 3.1, 3.2, 3.3
- states: `pending_exit=false`, `pending_exit=true`, `process_exited`, `source_ids_stored`, `panic_hook_installed`, `panic_recorded`, `previous_hook_ran`, `stale_pid_present`, `stale_pid_absent`, `reap_skipped`, `record_before_session`
- transitions:
  - `SIGTERM|SIGINT|SIGHUP while pending_exit=false` → `pending_exit=true` → **set** — anchor '            AppMsg::TrayQuit => {' — the handler emits the same message the tray quit uses, and logs the signal
  - `a second signal while pending_exit=true` → `process_exited` → **forced** — proposal sentence: 'a second signal while `pending_exit` is set exits immediately so a wedged stop cannot make the app unkillable'
  - `init_logging returns during try_run` → `panic_hook_installed` → **set** — anchor '    crate::logging::init_logging(&paths);' — install_panic_hook() is called immediately after
  - `startup with paths.pid_file_path() present` → `stale_pid_present` → **set** — anchor '                    if !skip_orphans && let Err(err) = cleanup_orphaned_backend(&orphan_paths) {' — the record is appended before the reap
  - `startup without a PID file` → `stale_pid_absent` → **no-op** — design sentence: 'The PID file is removed on every clean stop and crash exit'
  - `settings failed to load (skip_orphans = true)` → `reap_skipped` → **forced** — anchor '            let skip_orphans = settings_load_error.is_some();' — no reap and no record
- forbidden:
  - a tokio signal task instead of the GLib handler — the quit path owns GTK state and must run on the main loop
  - a signal path that destroys the window without stopping the backend (it must reuse App::quit, so the exit record is written)
  - a panic hook that replaces the previous hook instead of chaining it
  - writing the unclean-previous-run record after `check_and_kill_orphaned` has removed the PID file, or from outside the lifecycle-lock block
  - holding the short-lived RotatingFileWriter beyond the record
- seeding:
  - panic record: call the pure `fn panic_record(info: &std::panic::PanicHookInfo) -> String` directly from a test that triggers it through `std::panic::catch_unwind`, and drive `AppLogger` with the existing `log_record` harness — do not install the global hook in a unit test
  - the signal path has no unit seeding: it is manual verification (`kill -TERM` while connected, task 5.2)
  - Seed the stale PID file with a LIVE but non-matching pid: `PidFile::new(paths.pid_file_path()).write(std::process::id(), Path::new("/bin/sh"), &temp_config)`. `PidFile::write` builds the record through `process_start_time(pid)?`, so a dead pid returns Err(NotFound) and the test fails before asserting; serialising a PidOwnershipRecord directly is unavailable because crates/ui has no serde_json dependency. process_matches_record then fails, check_and_kill_orphaned removes the file and returns false, and the record reads `killed=false` with the pid file present before the reap.
- budgets:
  - 3 signals handled: SIGTERM, SIGINT, SIGHUP
  - exactly 1 unclean-previous-run record per app start
  - exactly 1 panic record per panic
  - backend.log writer opened at DEFAULT_MAX_BYTES, the same cap the connection task uses

**`status-text`** — tasks 4.1
- states: `origin=User`, `origin=AutoReconnect`, `origin=Restart`, `prev=Running`, `prev=Stopped`, `text=Connected`, `text=RestartingAfterCrash`, `text=Reconnecting`, `text=Connecting`, `text=unchanged`, `status_not_updated`, `attempt_preserved`
- transitions:
  - `state=Running with metadata` → `text=Connected` → **no-op** — anchor '                ("Connected".to_string(), details)' — details string unchanged
  - `state=Starting with prev=Running` → `text=RestartingAfterCrash` → **set** — anchor '        let from = self.process_state.clone();' in apply_state — the previous state is already captured there
  - `state=Starting, prev != Running, origin=AutoReconnect` → `text=Reconnecting` → **set** — anchor '        self.auto_reconnect_attempts += 1;' in schedule_auto_reconnect — n is the attempt, denominator MAX_AUTO_RECONNECTS
  - `state=Starting, prev != Running, origin=User or origin=Restart` → `text=Connecting` → **no-op** — anchor '            (ProcessState::Starting, _) => ("Connecting…".to_string(), "Resolving nodes".into()),'
  - `start_connection called (new generation)` → `prev=Stopped` → **forced** — anchor '        self.apply_state(&ProcessState::Starting);' in start_connection — a fresh connect is not a respawn; origin is set from its new parameter
  - `state=Stopping, Stopped or Error` → `text=unchanged` → **no-op** — anchor '            (ProcessState::Stopping, _) => {'
  - `a superseded generation reports Starting` → `status_not_updated` → **no-op** — anchor '                if !is_current_generation(generation, self.connection_generation) {'
  - `connect with origin=AutoReconnect` → `attempt_preserved` → **no-op** — anchor 'fn cancels_auto_reconnect(origin: ConnectOrigin) -> bool {' — `origin != ConnectOrigin::AutoReconnect`, so the counter is not reset
- forbidden:
  - two pure status functions over the same widget pair — one `status_texts(&StatusView) -> (String, String)` only (this supersedes tasks.md's `status_primary` name, per the sprint's one-status-function decision)
  - 'Restarting after crash' shown for a fresh connect or for a state reported by a superseded generation
  - 'Reconnecting (n/3)' with n read from anything but the single attempt accessor
  - reading the health/dns fields in a non-Running arm (they belong to the sibling change and `App` clears them outside Running)
- seeding:
  - `status_texts` is a free function over `StatusView` — build the struct literally in a table test beside `toggle_disabled_while_stopping`; app.rs tests never construct an App or a GTK widget
  - connection metadata for the Running arms: the existing `snapshot` / `session_target_node` fixtures
  - prev_state is reached only through `apply_state`'s captured `from` in production; a test sets the field on `StatusView` directly
- budgets:
  - MAX_AUTO_RECONNECTS = 3 is the denominator printed in 'Reconnecting (n/3)'
  - n ranges over 1..=3 — attempt 0 with origin AutoReconnect must still render 'Connecting…'

**`verification`** — tasks 5.1, 5.2
- states: `floor_green`, `live_verified`
- transitions:
  - `make fmt && make clippy && make test TEST_TIMEOUT=10m` → `floor_green` → **set** — Makefile anchors 'fmt:', 'clippy:' and 'test:' — TEST := timeout $(TEST_TIMEOUT) $(CARGO) test, TEST_ARGS := -- --test-threads=$(TEST_THREADS)
  - `live: connect, apply-with-restart, node switch, disconnect, quit` → `live_verified` → **set** — spec scenario 'Stop reasons are distinguished' — the four exit records read apply-restart, node-switch, user-stop, app-quit
  - `live: kill -TERM while connected` → `live_verified` → **set** — tasks.md 3.1: 'verified live' — app log records the signal, backend.log has reason=app-quit, TUN device gone
- forbidden:
  - a bare `cargo test` without a timeout and a thread cap (the repo's own test-limits rule)
  - claiming 5.2 done from unit tests — it needs a real backend and a real TUN device
- seeding:
  - floor: run from the repo root against a clean working tree
  - live: a real xray or sing-box binary with TUN granted; not reachable in CI
- budgets:
  - workspace tests: timeout 10m, --test-threads=4
  - per-crate tests: timeout 5m, --test-threads=4

### Floor

make fmt && make clippy && make test TEST_TIMEOUT=10m   (fmt = cargo fmt -- --check; clippy = cargo clippy --workspace --all-targets --all-features -- -D warnings; test = timeout 10m cargo test --workspace --all-targets -- --test-threads=4). Plus the MANUAL task 5.2 live pass: connect, apply-with-restart, node switch, disconnect, quit, and a kill -TERM while connected.


### Requirements map

- ### Requirement: Backend output survives the application The system SHALL append every line the backend writes to stdout or stderr, and every line the route helper writes — including during a TUN rout…
  - tests: `crates/process/src/manager.rs::tests::exit_record_marks_requested_stop`; `crates/process/src/manager.rs::tests::last_error_survives_access_noise`
- - **THEN** `<state_dir>/logs/backend.log` SHALL still contain the backend's last output lines and an exit record marking the exit as unrequested with reason `crash`
  - tests: `crates/process/src/manager.rs::tests::exit_record_marks_unrequested_crash`
- - **THEN** every line SHALL still be written to the backend log file
  - tests: `crates/process/src/manager.rs::tests::last_error_survives_access_noise`
- - **THEN** every line the route helper printed and the pass's outcome SHALL appear in `<state_dir>/logs/backend.log`, and none SHALL be written only to the application's standard error
  - tests: `crates/ui/src/app.rs::tests::stale_pid_file_records_unclean_previous_run`; `MANUAL task 3.1: startup recovery pass output lands in backend.log`
- - **THEN** the four exit records SHALL carry reasons `user-stop`, `apply-restart`, `node-switch`, and `app-quit` respectively
  - tests: `crates/process/src/manager.rs::tests::exit_record_reason_per_stop_reason`; `crates/ui/src/app.rs::tests::stop_reason_for_maps_every_input`; `crates/ui/src/connection.rs::tests::stop_reason_reaches_the_exit_record`
- - **THEN** the exit record SHALL carry reason `start-failed`
  - tests: `crates/process/src/manager.rs::tests::exit_record_marks_start_failed_on_tun_device_timeout`
- - **THEN** the exit record SHALL carry `requested=false`, reason `start-failed`, and `crashes_in_window=0`
  - tests: `crates/process/src/manager.rs::tests::exit_before_ready_records_start_failed (created by THIS change; #1's startup_failure_writes_one_session_and_no_crash predates `reason=` and asserts the session count and crashes_in_window=0 only)`
- - **THEN** the exit record SHALL carry that warning line as `last_error` and the final access line as `last_output`
  - tests: `crates/process/src/manager.rs::tests::last_error_survives_access_noise`; `crates/process/src/manager.rs::tests::last_error_is_none_without_warnings`; `crates/process/src/manager.rs::tests::last_error_matches_through_ansi`
- - **THEN** the session record SHALL state `hijack=hijack`, `capture_dns=true`, `nodes_pinned=true`, and `profile=app`
  - tests: `crates/ui/src/connection.rs::tests::xray_tun_session_record_carries_dns_decisions`; `crates/process/src/manager.rs::tests::session_record_appends_caller_fields`; `crates/core/src/models/imported_profile.rs::tests::uses_imported_profile_matches_resolve_effective_config`
- - **THEN** `backend.log` SHALL contain a record stating the previous run ended without stopping its backend, before any new session record
  - tests: `crates/ui/src/app.rs::tests::stale_pid_file_records_unclean_previous_run`; `crates/ui/src/app.rs::tests::no_pid_file_records_nothing`
- ### Requirement: Connection decisions are logged The application log SHALL record, at `info` level or above: each connection attempt with its origin (`user`, `node`, `auto-reconnect`, or `restart`), t…
  - tests: `crates/ui/src/connection.rs::tests::failover_is_traceable`; `crates/ui/src/app.rs::tests::connect_line`; `crates/ui/src/app.rs::tests::auto_reconnect_line`; `crates/ui/src/app.rs::tests::quit_line`; `crates/process/src/state.rs::tests::rejected_transition_changes_nothing`
- - **THEN** the app log SHALL contain the connect record with `candidates=3`, a failure record for candidate 1 with its reason, and a start record for candidate 2, in that order
  - tests: `crates/ui/src/connection.rs::tests::failover_is_traceable`
- - **THEN** the app log SHALL contain a record with the attempt number and the limit of 3, followed by a connect record with origin `auto-reconnect` when it fires
  - tests: `crates/ui/src/app.rs::tests::auto_reconnect_line`; `crates/ui/src/app.rs::tests::auto_reconnect_exhausted_line`
- - **THEN** the app log SHALL contain a crash-respawn record with the exit code or signal and the crash count, and state-transition records for `Running` → `Starting` → `Running`
  - tests: `crates/process/src/manager.rs::tests::crash_respawn_records_crash_count`; `crates/process/src/state.rs::tests::rejected_transition_changes_nothing`
- ### Requirement: Status bar distinguishes restarts from connects While a connection is starting, the status bar SHALL distinguish why: an in-place respawn after the backend exited unexpectedly SHALL s…
  - tests: `crates/ui/src/app.rs::tests::status_texts_covers_every_starting_case`
- - **THEN** the status bar SHALL show "Restarting after crash" until the backend is running again or the connection ends
  - tests: `crates/ui/src/app.rs::tests::status_texts_covers_every_starting_case`
- - **THEN** the status bar SHALL show "Reconnecting (2/3)"
  - tests: `crates/ui/src/app.rs::tests::status_texts_covers_every_starting_case`
- - **THEN** the status bar SHALL show "Connecting…"
  - tests: `crates/ui/src/app.rs::tests::status_texts_covers_every_starting_case`
- ### Requirement: Termination signals and panics leave a record The application SHALL handle SIGTERM, SIGINT, and SIGHUP by running the same quit path as a user quit, so a running backend is stopped, i…
  - tests: `MANUAL task 3.1: kill -TERM while connected — app log records the signal, backend.log carries reason=app-quit, TUN device gone`
- - **THEN** the app log SHALL record the signal, the backend SHALL be stopped gracefully, and `backend.log` SHALL contain an exit record with reason `app-quit`
  - tests: `crates/ui/src/app.rs::tests::stop_reason_for_maps_every_input`; `MANUAL task 3.1 live signal pass`
- - **THEN** the app log file SHALL contain an `error` record with the panic message and source location
  - tests: `crates/ui/src/logging.rs::tests::panic_record_names_message_and_location`; `crates/ui/src/logging.rs::tests::install_panic_hook_chains_previous`

### Plan review

`pass` by zarchitect, 2 rounds.

- Round 1 raised 5 blockers, all fixed: the pre-ready exit record had no owner (l1 now emits requested=false reason=start-failed from the readiness path, never through graceful_stop); l2 declared one crate while writing three; the traceability test filtered a process-wide log capture on addresses eight concurrent tests share; the stale-PID seeding could not run (PidFile::write goes through process_start_time(pid)?); and a requirement pointed at an optional test.
- Round 2 raised 2 residuals, both fixed and grep-verified: the new transition row pointed at state exit_record_crash (states[] gained exit_record_start_failed and the row now names it), and the requirement for `requested=false reason=start-failed crashes_in_window=0` still mapped to #1's test, which predates `reason=` (now mapped to exit_before_ready_records_start_failed, created by this change).
- Round 2 also found four reported-but-unapplied edits, now applied and verified: the xray_on_lo TUN-timeout seeding (lo always exists, so the timeout never fires), two stale l5 seeding lines over 203.0.113.x, seeding not narrowed per chunk for l6/l7, and an unused PidOwnershipRecord re-export. The duplicate l1 anchor is distinguished.
- Accepted coverage gap (reviewer confirmed not a blocker): the process crate has no capturing logger, so 'each backend state transition' is implemented but has no automated assertion; l4's verify is green.
- Accepted (reviewer confirmed not a blocker): two contract EVIDENCE anchors are not verbatim at HEAD (truncate_reason and exit_status_field are module-level and unindented). Every sites[] anchor greps to exactly one line.
- l6's second-signal force exit is now a pure force_exit_on_second_signal(pending_exit) guard with a table test and a site in App::quit — App::quit has no pending_exit early return today.

## Plan appendix

```json
{
  "v": 2,
  "change": "log-connection-decisions",
  "baseSha": "1d6a0fb8eb02e73a65fc393adfdd743e2bba742f",
  "generatedAt": "2026-09-16T09:32:50.022357+00:00",
  "tier": "heavy",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality",
    "arch"
  ],
  "chunks": [
    {
      "id": "l1",
      "taskIds": [
        "1.1",
        "1.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "exit-records",
      "shard": "process",
      "pkgDirs": [
        "crates/process/src"
      ],
      "pkgs": [
        "v2ray-rs-process"
      ],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "StopReason (new enum) + stop_reason field on ProcessManager",
          "anchor": "const REASON_MAX_CHARS: usize = 200;",
          "change": "add `pub enum StopReason { UserStop, NodeSwitch, ApplyRestart, AppQuit, StartFailed }` with a `as_str()`/Display yielding user-stop|node-switch|apply-restart|app-quit|start-failed; store the pending reason on the manager (default UserStop)"
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::new",
          "anchor": "            cached_version: None,",
          "change": "initialize the new stop_reason field alongside the existing ones"
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::shutdown",
          "anchor": "    pub async fn shutdown(&mut self) {",
          "change": "add `shutdown_with(&mut self, reason: StopReason)` that sets the stored reason then runs the current body; keep `shutdown()` delegating with UserStop"
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::stop",
          "anchor": "    pub async fn stop(&mut self) -> Result<(), ProcessError> {",
          "change": "stop() keeps the reason already stored by shutdown_with; no signature change unless a stop_with is needed for the Error-without-child path"
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "launch (TUN device timeout)",
          "anchor": "                return Err(ProcessError::TunDeviceTimeout(rt.iface.clone()));",
          "change": "set stop_reason = StartFailed before the preceding graceful_stop() so the exit record reads reason=start-failed"
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "launch (xray-up failure)",
          "anchor": "                return Err(ProcessError::TunHelper(e));",
          "change": "same: StartFailed before the graceful_stop() on helper failure"
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "write_exit_record",
          "anchor": "    fn write_exit_record(&self, requested: bool, status: Option<&ExitStatus>) {",
          "change": "print `reason=` after `requested=`: the stored StopReason for requested stops, literal `crash` for requested=false; keep requested/crashes_in_window/last_output"
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "graceful_stop",
          "anchor": "        self.write_exit_record(true, status.as_ref());",
          "change": "pass the stored reason through (or read it inside write_exit_record)"
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "handle_unexpected_exit / wait_and_handle_exit crash records",
          "anchor": "        self.write_exit_record(false, Some(&status));",
          "change": "unrequested path prints reason=crash; the Wait-error record at `self.write_exit_record(false, None);` likewise"
        },
        {
          "task": "1.1",
          "file": "crates/process/src/lib.rs",
          "symbol": "crate exports",
          "anchor": "pub use manager::{ProcessError, ProcessManager};",
          "change": "re-export StopReason so crates/ui can name it"
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "tests::exit_record_marks_requested_stop / exit_record_marks_unrequested_crash",
          "anchor": "    async fn exit_record_marks_requested_stop() {",
          "change": "extend with per-reason assertions; add stub tests for TUN device timeout → reason=start-failed and crash → requested=false reason=crash"
        },
        {
          "task": "1.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "last_error_line (new, beside last_output_line)",
          "anchor": "    fn last_output_line(&self) -> Option<String> {",
          "change": "add `fn last_error_line(&self) -> Option<String>` scanning the same last_n(50) for [Warning]/[Error]/WARN/ERROR/FATAL/panic tolerating ANSI; last_output_line unchanged"
        },
        {
          "task": "1.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "write_exit_record",
          "anchor": "                \"requested={requested} {} crashes_in_window={} last_output={last}\",",
          "change": "append ` last_error=` with last_error_line() or `none`"
        },
        {
          "task": "1.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "tests (new: last error survives access noise)",
          "anchor": "    async fn exit_record_marks_unrequested_crash() {",
          "change": "add a stub that prints a [Warning] then access lines → last_error is the warning, last_output the access line; assert the crash Error text is unchanged"
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "readiness-failure exit path (added by confirm-backend-ready-before-running)",
          "anchor": "        self.write_exit_record(true, status.as_ref());",
          "change": "After the rebase onto confirm-backend-ready-before-running there are THREE callers, not two: graceful_stop (requested=true, self.stop_reason.as_str()), handle_unexpected_exit and wait_and_handle_exit (requested=false, CRASH_REASON), and the readiness path (requested=false, StopReason::StartFailed.as_str(), crashes_in_window=0). The readiness path writes its own record and must not route through graceful_stop, which would emit requested=true reason=user-stop."
        }
      ],
      "contract": {
        "states": [
          "stop_reason=UserStop",
          "stop_reason=NodeSwitch",
          "stop_reason=ApplyRestart",
          "stop_reason=AppQuit",
          "stop_reason=StartFailed",
          "exit_record_requested",
          "exit_record_crash",
          "exit_record_absent",
          "last_error=line",
          "last_error=none",
          "error_text_unchanged",
          "exit_record_start_failed"
        ],
        "transitions": [
          {
            "input": "ProcessManager::new(..) constructed",
            "state": "stop_reason=UserStop",
            "effect": "set",
            "evidence": "anchor '            cached_version: None,' in ProcessManager::new — the new `stop_reason` field is initialized beside it"
          },
          {
            "input": "shutdown() with no explicit reason",
            "state": "stop_reason=UserStop",
            "effect": "set",
            "evidence": "anchor '    pub async fn shutdown(&mut self) {' — shutdown() delegates to shutdown_with(StopReason::UserStop)"
          },
          {
            "input": "shutdown_with(StopReason::NodeSwitch)",
            "state": "stop_reason=NodeSwitch",
            "effect": "set",
            "evidence": "anchor '    pub async fn shutdown(&mut self) {' — shutdown_with stores the reason before running today's body"
          },
          {
            "input": "shutdown_with(StopReason::ApplyRestart)",
            "state": "stop_reason=ApplyRestart",
            "effect": "set",
            "evidence": "anchor '    pub async fn shutdown(&mut self) {'"
          },
          {
            "input": "shutdown_with(StopReason::AppQuit)",
            "state": "stop_reason=AppQuit",
            "effect": "set",
            "evidence": "anchor '    pub async fn shutdown(&mut self) {'"
          },
          {
            "input": "stop() called without a preceding shutdown_with",
            "state": "stop_reason=UserStop",
            "effect": "no-op",
            "evidence": "anchor '    pub async fn stop(&mut self) -> Result<(), ProcessError> {' — stop takes no reason parameter and leaves the stored reason alone"
          },
          {
            "input": "TUN device did not appear within tun::DEVICE_TIMEOUT during launch",
            "state": "stop_reason=StartFailed",
            "effect": "forced",
            "evidence": "anchor '                return Err(ProcessError::TunDeviceTimeout(rt.iface.clone()));' — set before the graceful_stop() that precedes it"
          },
          {
            "input": "xray-up helper returned Err during launch",
            "state": "stop_reason=StartFailed",
            "effect": "forced",
            "evidence": "anchor '                return Err(ProcessError::TunHelper(e));' — set before the preceding graceful_stop()"
          },
          {
            "input": "graceful_stop completes (requested stop)",
            "state": "exit_record_requested",
            "effect": "set",
            "evidence": "anchor '        self.write_exit_record(true, status.as_ref());' at the tail of graceful_stop — prints `requested=true reason=<stop_reason.as_str()>`"
          },
          {
            "input": "child exited while state was Running",
            "state": "exit_record_crash",
            "effect": "forced",
            "evidence": "anchor '        self.write_exit_record(false, Some(&status));' in handle_unexpected_exit — prints `requested=false reason=crash`"
          },
          {
            "input": "child.wait() returned Err",
            "state": "exit_record_crash",
            "effect": "forced",
            "evidence": "anchor '                self.write_exit_record(false, None);' in wait_and_handle_exit — prints `requested=false reason=crash code=none`"
          },
          {
            "input": "log_writer is None",
            "state": "exit_record_absent",
            "effect": "no-op",
            "evidence": "anchor '    fn write_exit_record(&self, requested: bool, status: Option<&ExitStatus>) {' — its first statement returns when log_writer.is_none()"
          },
          {
            "input": "buffered output holds a [Warning] line followed by access lines",
            "state": "last_error=line",
            "effect": "set",
            "evidence": "anchor '    fn last_output_line(&self) -> Option<String> {' — last_error_line scans the same buffer.last_n(50); last_output stays the access line"
          },
          {
            "input": "no matching warning/error token in the last 50 lines",
            "state": "last_error=none",
            "effect": "set",
            "evidence": "requirement sentence: 'the last warning, error, or fatal line among the recent output (or `none`)'"
          },
          {
            "input": "matching line wrapped in CSI colour codes",
            "state": "last_error=line",
            "effect": "set",
            "evidence": "tasks.md 1.2: '(`[Warning]`, `[Error]`, `WARN`, `ERROR`, `FATAL`, `panic`, ANSI-tolerant)'"
          },
          {
            "input": "crash exit with a last stderr line",
            "state": "error_text_unchanged",
            "effect": "no-op",
            "evidence": "anchor '        if let Some(reason) = self.last_output_line() {' in handle_unexpected_exit — last_output_line is not modified"
          },
          {
            "input": "child exited before ready (readiness path)",
            "state": "exit_record_start_failed",
            "effect": "forced",
            "evidence": "tasks.md 1.1: 'a backend that exits before ready records requested=false reason=start-failed crashes_in_window=0 from the readiness path, never through graceful_stop'"
          }
        ],
        "forbidden": [
          "`reason=crash` together with `requested=true` — a requested stop is never a crash",
          "`reason=user-stop` on any launch-failure path (TunDeviceTimeout, TunHelper)",
          "write_exit_record accepting a `StopReason` for the unrequested path — `crash` is not a StopReason variant and must come from the single `CRASH_REASON` constant",
          "changing `last_output_line`'s selection rule or the crash `Error` message text",
          "more than one exit record per child exit"
        ],
        "seeding": [
          "requested stop with a given reason: `mgr.shutdown_with(StopReason::NodeSwitch).await` (or the matching variant) after a successful `mgr.start().await`",
          "crash record: `manager_for(&dir, \"...exit 3\")` + `mgr.set_auto_restart(false)` + `mgr.wait_and_handle_exit().await`, exactly as `exit_record_marks_unrequested_crash` does",
          "last_error: stub body prints a `[Warning] ...` line, then access-shaped stdout lines, then exits — read the record via `wait_for_lines(&dir.path().join(\"backend.log\"), 2)`",
          "records are read from the file with `read_lines` / `wait_for_lines`, never from the broadcast channel",
          "Seed `exit_record_marks_start_failed_on_tun_device_timeout` with a stub helper whose `xray-up` exits 1 (ProcessError::TunHelper), which is instant. Do NOT seed it with `xray_on_lo`: its iface is `lo`, which always exists, so wait_for_device returns immediately and the timeout never fires. The nonexistent-iface variant works but costs a fixed tun::DEVICE_TIMEOUT of 10 s that no manager field overrides."
        ],
        "budgets": [
          "last_error_line scans exactly the last 50 buffered lines (`buffer.last_n(50)`), the same window as last_output_line",
          "reason and last_error text pass through `truncate_reason`: REASON_MAX_CHARS = 200 chars, newline-scrubbed",
          "`wait_for_lines` polls up to 2 s for the expected line count",
          "per-test wall clock: the seam's verify command caps the whole crate at 5 m",
          "tun::DEVICE_TIMEOUT = 10 s, fixed and not per-manager overridable — the reason the helper-failure seeding is preferred"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST: add `exit_before_ready_records_start_failed` — FATAL-then-exit stub with a ready probe (the harness confirm-backend-ready-before-running adds), assert the exit record carries `requested=false reason=start-failed crashes_in_window=0` and that no `reason=user-stop` record is written",
        "TEST FIRST: extend `exit_record_marks_requested_stop` to assert `reason=user-stop`, and add `exit_record_reason_per_stop_reason` driving shutdown_with for NodeSwitch/ApplyRestart/AppQuit and asserting `reason=node-switch|apply-restart|app-quit`",
        "TEST FIRST: add `exit_record_marks_start_failed_on_tun_device_timeout` using `stub_helper` with an xray-up that exits 1 (ProcessError::TunHelper — instant). Do NOT use `xray_on_lo`: its iface is `lo`, which always exists, so wait_for_device returns immediately and the timeout never fires; the nonexistent-iface variant costs a fixed tun::DEVICE_TIMEOUT of 10 s that no manager field overrides. Assert `requested=true reason=start-failed`",
        "TEST FIRST: extend `exit_record_marks_unrequested_crash` to assert `requested=false reason=crash`",
        "TEST FIRST: add `last_error_survives_access_noise` — stub prints `[Warning] cert about to expire`, then two access-shaped stdout lines, then exits; assert `last_error=[Warning] cert about to expire` and `last_output=<final access line>`, and that the ProcessState::Error text still ends with the final buffered line (unchanged behavior)",
        "TEST FIRST: add `last_error_is_none_without_warnings` and `last_error_matches_through_ansi` (stub prints `\\033[33m[Warning]\\033[0m tls retry`)",
        "Add `pub enum StopReason { UserStop, NodeSwitch, ApplyRestart, AppQuit, StartFailed }` (Debug, Clone, Copy, PartialEq, Eq) with `pub fn as_str(&self) -> &'static str` yielding user-stop|node-switch|apply-restart|app-quit|start-failed and `impl std::fmt::Display` delegating to as_str, beside `const REASON_MAX_CHARS: usize = 200;` in crates/process/src/manager.rs; add `const CRASH_REASON: &str = \"crash\";` there too",
        "Add field `stop_reason: StopReason` to `ProcessManager`, initialized to `StopReason::UserStop` next to `cached_version: None,` in `ProcessManager::new`",
        "Add `pub async fn shutdown_with(&mut self, reason: StopReason)` holding today's `shutdown` body preceded by `self.stop_reason = reason;`; `shutdown` becomes `self.shutdown_with(StopReason::UserStop).await`",
        "Set `self.stop_reason = StopReason::StartFailed;` immediately before the `graceful_stop()` calls guarding `ProcessError::TunDeviceTimeout` and `ProcessError::TunHelper` in `launch`",
        "Change the signature to `fn write_exit_record(&self, requested: bool, reason: &str, status: Option<&ExitStatus>)` and emit `requested={requested} reason={reason} {status} crashes_in_window={} last_output={last} last_error={err}`; graceful_stop passes `self.stop_reason.as_str()`, both unrequested sites pass `CRASH_REASON`",
        "Add `fn last_error_line(&self) -> Option<String>` beside `last_output_line`, scanning `buffer.last_n(50)` in reverse for a line whose ANSI-stripped content contains any of `[Warning]`, `[Error]`, `WARN`, `ERROR`, `FATAL`, `panic`, returning it through `truncate_reason`; add the private helper `fn without_ansi(line: &str) -> String` next to `truncate_reason`",
        "Re-export `StopReason` from crates/process/src/lib.rs at anchor 'pub use manager::{ProcessError, ProcessManager};'",
        "Emit the readiness-failure exit record from the readiness path itself with `write_exit_record(false, StopReason::StartFailed.as_str(), Some(&status))`; do NOT let it fall through to graceful_stop, whose stored `stop_reason` defaults to UserStop and would emit `requested=true reason=user-stop` — banned by this seam's own forbidden list",
        "Reset `self.stop_reason = StopReason::UserStop;` after `write_exit_record` so a launch failure's StartFailed does not leak into a later stop on the same manager"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-process && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "l2",
      "taskIds": [
        "1.3"
      ],
      "prev": "l1",
      "sharedPkg": "crates/process/src",
      "parallel": false,
      "seam": "session-fields",
      "shard": "process",
      "pkgDirs": [
        "crates/process/src",
        "crates/core/src/models",
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-core",
        "v2ray-rs-process",
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "1.3",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::with_log_file / with_session_fields",
          "anchor": "    pub fn with_log_file(mut self, writer: Option<Arc<RotatingFileWriter>>) -> Self {",
          "change": "add builder `with_session_fields(mut self, fields: String) -> Self` storing an Option<String>"
        },
        {
          "task": "1.3",
          "file": "crates/process/src/manager.rs",
          "symbol": "write_session_record",
          "anchor": "            &format!(\"backend={backend} version={version} node={node} tun={tun}\"),",
          "change": "append the caller-supplied session fields (truncate_reason-sanitized) after tun="
        },
        {
          "task": "1.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "candidate loop manager build",
          "anchor": "            let mut mgr = configure(",
          "change": "chain .with_session_fields(format!(\"hijack={} capture_dns={} strict={} nodes_pinned={} profile={}\", ...)) from effective_settings.tun.dns_hijack, the built TunRuntime's capture_dns/strict, `pinned`, and whether resolve_effective_config used an imported profile"
        },
        {
          "task": "1.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "resolve_effective_config call (profile override detection)",
          "anchor": "            let (mut effective_rules, mut effective_settings) = resolve_effective_config(",
          "change": "capture whether the candidate's routing/DNS came from an imported profile → the `profile=imported|app` field (resolve_effective_config currently returns no such flag — see risks)"
        },
        {
          "task": "1.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests::live_connect_writes_backend_diagnostics",
          "anchor": "            contents.contains(\"backend=sing-box version=1.13.0 node=203.0.113.1 tun=off\"),",
          "change": "existing assertion must absorb the new suffix; add an xray TUN stub test asserting the session line's hijack/capture_dns/strict/nodes_pinned/profile fields"
        },
        {
          "task": "1.3",
          "file": "crates/core/src/models/imported_profile.rs",
          "symbol": "uses_imported_profile (new)",
          "anchor": "pub fn resolve_effective_config(",
          "change": "Add `pub fn uses_imported_profile(node_ref: &ConnectionNodeRef, subscriptions: &[Subscription]) -> bool` holding the exact condition of this function's first arm, and make that arm read through it so the predicate and the resolution cannot drift."
        },
        {
          "task": "1.3",
          "file": "crates/core/src/models/mod.rs",
          "symbol": "imported_profile re-export",
          "anchor": "pub use imported_profile::{ImportedProfile, resolve_effective_config};",
          "change": "Extend the re-export with `uses_imported_profile`."
        }
      ],
      "contract": {
        "states": [
          "session_fields=None",
          "session_fields=Some(text)",
          "session_record_single_line",
          "session_record_repeated",
          "tun_runtime=None",
          "capture_dns=true",
          "strict=false",
          "profile=imported",
          "profile=app"
        ],
        "transitions": [
          {
            "input": "manager built without with_session_fields",
            "state": "session_fields=None",
            "effect": "no-op",
            "evidence": "anchor '            &format!(\"backend={backend} version={version} node={node} tun={tun}\"),' in write_session_record — the line ends at tun="
          },
          {
            "input": "with_session_fields(text) chained in the builder",
            "state": "session_fields=Some(text)",
            "effect": "set",
            "evidence": "anchor '    pub fn with_log_file(mut self, writer: Option<Arc<RotatingFileWriter>>) -> Self {' — the builder shape with_session_fields copies"
          },
          {
            "input": "field text containing \\n or \\r",
            "state": "session_record_single_line",
            "effect": "forced",
            "evidence": "anchor '    fn truncate_reason(line: &str) -> String {' comment 'Records are single-line'"
          },
          {
            "input": "respawn after a crash",
            "state": "session_record_repeated",
            "effect": "set",
            "evidence": "anchor '        self.write_session_record().await;' — the first statement of launch, which respawn also calls"
          },
          {
            "input": "TUN disabled or backend v2ray",
            "state": "tun_runtime=None",
            "effect": "forced",
            "evidence": "anchor '    if !settings.tun.enabled {' in build_tun_runtime — fields read hijack=off capture_dns=false strict=false"
          },
          {
            "input": "xray + tun.enabled + dns_hijack Hijack + every node hostname pinned",
            "state": "capture_dns=true",
            "effect": "set",
            "evidence": "anchor '        capture_dns: backend == BackendType::Xray' in build_tun_runtime"
          },
          {
            "input": "drop_strict_route decided for the connection",
            "state": "strict=false",
            "effect": "forced",
            "evidence": "anchor '            if drop_strict_route {' inside the candidate loop — effective_settings.tun.strict_route is cleared before build_tun_runtime"
          },
          {
            "input": "candidate's subscription has use_imported_profile and an imported_profile",
            "state": "profile=imported",
            "effect": "set",
            "evidence": "anchor '        && sub.use_imported_profile' in resolve_effective_config"
          },
          {
            "input": "manual node, or subscription without an imported profile",
            "state": "profile=app",
            "effect": "set",
            "evidence": "anchor '    (global_rules.to_vec(), settings.clone())' — the fallback arm of resolve_effective_config"
          }
        ],
        "forbidden": [
          "computing `profile=` by re-deriving the imported-profile condition inline in connection.rs — it must call the single core predicate",
          "reading `settings.tun.*` rather than `effective_settings.tun.*` for the session fields (the imported profile and the strict-route drop change them)",
          "the session fields naming a node's credentials, address, or port — only the five keys",
          "more than one session record per launch"
        ],
        "seeding": [
          "session_fields=Some: build through the production builder chain in connection.rs, or in process tests `manager_for(..).with_log_file(Some(backend_log(dir.path()))).with_session_fields(\"hijack=hijack capture_dns=true\".into())`",
          "tun_runtime=Some(rt) in a ui test: `tun_settings()` (or `strict_route_settings()`) passed to `connect(&stub, ..)` with the `capless_probe` configure hook",
          "profile=imported: a `Subscription` fixture with `use_imported_profile = true` and `imported_profile = Some(..)`, reached through `resolve_effective_config` — never by setting the field on the record",
          "read the session line from `stub.paths.logs_dir().join(\"backend.log\")` after the first Running state, as `live_connect_writes_backend_diagnostics` does"
        ],
        "budgets": [
          "exactly 1 session record per launch (`contents.matches(\" session \").count() == 1` for a single-candidate connect)",
          "session field text truncated at REASON_MAX_CHARS = 200",
          "ui connect tests wait for a state at most 20 s (`next_state`)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST (core): add `uses_imported_profile_matches_resolve_effective_config` in crates/core/src/models/imported_profile.rs — true for a subscription node whose subscription has use_imported_profile + imported_profile, false for a manual node, false when use_imported_profile is off",
        "TEST FIRST (process): add `session_record_appends_caller_fields` — `with_session_fields(\"hijack=hijack capture_dns=true strict=false nodes_pinned=true profile=app\".into())`, assert the single session line contains `tun=off hijack=hijack` and the whole suffix; add `session_fields_cannot_forge_a_second_line` with an embedded \\n",
        "TEST FIRST (ui): update `live_connect_writes_backend_diagnostics` so the existing assertion absorbs the suffix (assert `contents.contains(\"backend=sing-box version=1.13.0 node=203.0.113.1 tun=off\")` still holds and the same line ends with `hijack=off capture_dns=false strict=false nodes_pinned=true profile=app`)",
        "TEST FIRST (ui): add `xray_tun_session_record_carries_dns_decisions` — stub xray backend, `tun_settings()` with `capless_probe`, assert the session line contains `tun=on hijack=hijack capture_dns=true strict=false nodes_pinned=true profile=app`",
        "Add `pub fn uses_imported_profile(node_ref: &ConnectionNodeRef, subscriptions: &[Subscription]) -> bool` in crates/core/src/models/imported_profile.rs holding the exact condition of resolve_effective_config's first arm; make resolve_effective_config's arm read through it so the two cannot drift; extend the re-export at anchor 'pub use imported_profile::{ImportedProfile, resolve_effective_config};'",
        "Add field `session_fields: Option<String>` to `ProcessManager` and builder `pub fn with_session_fields(mut self, fields: String) -> Self` following the with_log_file shape",
        "In `write_session_record`, append `truncate_reason(&fields)` after `tun={tun}` when session_fields is Some",
        "In crates/ui/src/connection.rs add `fn hijack_field(mode: DnsHijackMode) -> &'static str` (hijack|native|disabled) and chain `.with_session_fields(format!(\"hijack={} capture_dns={} strict={} nodes_pinned={} profile={}\", ..))` onto the builder at anchor '            let mut mgr = configure(' — values from the built `tun` runtime (None → `off`/`false`/`false`), `pinned`, and `uses_imported_profile(&candidate.node_ref, &subscriptions)` → `imported`|`app`"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-core && make test-process && make test-ui && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "l3",
      "taskIds": [
        "2.1"
      ],
      "prev": "l2",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "seam": "stop-reason-plumbing",
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-process",
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "ConnectionCmd",
          "anchor": "enum ConnectionCmd {",
          "change": "`Stop(StopReason)`"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "ConnectionHandle::stop",
          "anchor": "        let _ = self.cmd_tx.try_send(ConnectionCmd::Stop);",
          "change": "take a StopReason parameter and send it"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "queued-stop check before start",
          "anchor": "        // A Stop that arrived while we were queued behind the previous",
          "change": "match `Ok(ConnectionCmd::Stop(_))`; nothing started yet so no shutdown_with"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "candidate-loop head stop check",
          "anchor": "'candidates: for candidate in candidates {",
          "change": "match Stop(reason) and pass it to parked.shutdown_with(reason)"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "select! stop during start_with_connection",
          "anchor": "            let started = tokio::select! {",
          "change": "Stop(reason) → mgr.shutdown_with(reason) and parked.shutdown_with(reason)"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "stop queued behind a successful start",
          "anchor": "                    // A Disconnect clicked while the start was in flight sits",
          "change": "match Stop(reason) → mgr.shutdown_with(reason)"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "supervision loop stop arm",
          "anchor": "                    Some(ConnectionCmd::Stop) = cmd_rx.recv() => {",
          "change": "the arm inside the `loop { tokio::select! {` that follows the comment `// The manager restarts in place on an unexpected exit; if` — Stop(reason) → mgr.shutdown_with(reason)"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "host-level failure teardown",
          "anchor": "                        if grant_fixable(&e) {",
          "change": "the mgr.shutdown()/parked.shutdown() above it become shutdown_with(StopReason::StartFailed)"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "stop_reason_for (new pure fn) + AppMsg::Disconnect arm",
          "anchor": "            AppMsg::Disconnect => {",
          "change": "map reconnect_pending→ApplyRestart, pending_direct_target→NodeSwitch, pending_exit→AppQuit, else UserStop; pass to handle.stop(reason) at `handle.stop();` inside DisconnectPlan::Stop"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::quit",
          "anchor": "    fn quit(&mut self, sender: &ComponentSender<Self>) {",
          "change": "QuitPlan::Stop calls handle.stop(StopReason::AppQuit)"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::ApplyAndRestart",
          "anchor": "            AppMsg::ApplyAndRestart => {",
          "change": "unchanged flag-setting; it is the ApplyRestart input to the mapping (reconnect_pending = true before Disconnect)"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::ConnectToNode node-switch path",
          "anchor": "                    self.pending_direct_target = Some(target);",
          "change": "the NodeSwitch input to the mapping; no behavior change beyond the reason"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "tests (new mapping unit test)",
          "anchor": "    fn reconnect_pending_consumed_on_stop_or_error() {",
          "change": "add a table test for stop_reason_for over (reconnect_pending, pending_direct_target, pending_exit) next to the existing pure-fn tests"
        }
      ],
      "contract": {
        "states": [
          "reason=UserStop",
          "reason=NodeSwitch",
          "reason=ApplyRestart",
          "reason=AppQuit",
          "reason=StartFailed",
          "reason=carried",
          "no_shutdown_call",
          "terminal_state_unchanged"
        ],
        "transitions": [
          {
            "input": "stop_reason_for(pending_exit=true, ..)",
            "state": "reason=AppQuit",
            "effect": "forced",
            "evidence": "anchor '    fn quit(&mut self, sender: &ComponentSender<Self>) {' — QuitPlan::Stop sets pending_exit then stops the handle"
          },
          {
            "input": "stop_reason_for(false, reconnect_pending=true, direct_target=None)",
            "state": "reason=ApplyRestart",
            "effect": "set",
            "evidence": "anchor '                self.reconnect_pending = true;' in the AppMsg::ApplyAndRestart arm, dispatched straight into AppMsg::Disconnect"
          },
          {
            "input": "stop_reason_for(false, false, direct_target=Some)",
            "state": "reason=NodeSwitch",
            "effect": "set",
            "evidence": "anchor '                    self.pending_direct_target = Some(target);' in the AppMsg::ConnectToNode arm, which sets reconnect_pending = false first"
          },
          {
            "input": "stop_reason_for(false, false, None)",
            "state": "reason=UserStop",
            "effect": "set",
            "evidence": "anchor '            AppMsg::Disconnect => {' — the plain Disconnect click"
          },
          {
            "input": "App::quit with QuitPlan::Stop",
            "state": "reason=AppQuit",
            "effect": "forced",
            "evidence": "anchor '                    handle.stop();' inside QuitPlan::Stop — becomes handle.stop(StopReason::AppQuit)"
          },
          {
            "input": "Stop received while the task is still queued behind the previous teardown",
            "state": "no_shutdown_call",
            "effect": "no-op",
            "evidence": "anchor '        // A Stop that arrived while we were queued behind the previous' — nothing has started yet; Stopped is reported"
          },
          {
            "input": "Stop received at the candidate-loop head",
            "state": "reason=carried",
            "effect": "set",
            "evidence": "anchor \"'candidates: for candidate in candidates {\" — passed to parked.shutdown_with(reason)"
          },
          {
            "input": "Stop received during start_with_connection",
            "state": "reason=carried",
            "effect": "set",
            "evidence": "anchor '            let started = tokio::select! {' — passed to mgr.shutdown_with and parked.shutdown_with"
          },
          {
            "input": "Stop queued behind a successful start",
            "state": "reason=carried",
            "effect": "set",
            "evidence": "anchor '                    // A Disconnect clicked while the start was in flight sits'"
          },
          {
            "input": "Stop received in the supervision loop",
            "state": "reason=carried",
            "effect": "set",
            "evidence": "anchor '                    Some(ConnectionCmd::Stop) = cmd_rx.recv() => {'"
          },
          {
            "input": "host-level start failure teardown",
            "state": "reason=StartFailed",
            "effect": "forced",
            "evidence": "anchor '                        if grant_fixable(&e) {' — the shutdown() calls above it"
          },
          {
            "input": "any Stop path",
            "state": "terminal_state_unchanged",
            "effect": "no-op",
            "evidence": "anchor '    fn relays(state: &ProcessState) -> bool {' — terminal states stay with the supervising loop"
          }
        ],
        "forbidden": [
          "`reason=UserStop` reaching the manager on a host-level start failure or a queued-behind-start teardown",
          "inferring the reason inside connection.rs from state rather than carrying it in the command",
          "a `handle.stop()` call site left without an explicit reason (the parameter is mandatory, no Default)",
          "changing which terminal state each Stop path reports"
        ],
        "seeding": [
          "stop_reason_for is a free function: call it directly with the three booleans/Option in a table test, next to `reconnect_pending_consumed_on_stop_or_error`",
          "connection-side reasons: drive `handle.stop(StopReason::NodeSwitch)` in a `connect(&stub, ..)` test and read `reason=node-switch` back from `stub.paths.logs_dir().join(\"backend.log\")`",
          "app.rs tests never construct an App or a GTK widget — the mapping must therefore stay a free function over plain inputs"
        ],
        "budgets": [
          "exactly 6 `ConnectionCmd::Stop` match sites in connection.rs after the change (queued-before-start, loop head, select! during start, queued-after-start, supervision loop, plus the host-level teardown that constructs StartFailed itself)",
          "cmd channel capacity stays 4 (`mpsc::channel::<ConnectionCmd>(4)`)",
          "ui connect tests wait at most 20 s per state (`next_state`)"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST (app.rs): add `stop_reason_for_maps_every_input` — a table over (pending_exit, reconnect_pending, direct_target.is_some()) asserting AppQuit / ApplyRestart / NodeSwitch / UserStop with the precedence above",
        "TEST FIRST (connection.rs): add `stop_reason_reaches_the_exit_record` — connect a stub, `handle.stop(StopReason::NodeSwitch)`, wait for Stopped, assert backend.log's exit record contains `reason=node-switch`",
        "Change `enum ConnectionCmd { Stop }` to `Stop(StopReason)` and `ConnectionHandle::stop(&self, reason: StopReason)` at anchor '        let _ = self.cmd_tx.try_send(ConnectionCmd::Stop);'",
        "Update all six Stop match sites per the transition table, passing the reason to `shutdown_with`; the host-level teardown at anchor '                        if grant_fixable(&e) {' uses `StopReason::StartFailed`",
        "Add `fn stop_reason_for(pending_exit: bool, reconnect_pending: bool, direct_target: bool) -> StopReason` in app.rs beside `disconnect_plan`, and call it in the DisconnectPlan::Stop arm at anchor '                            handle.stop();'",
        "`App::quit`'s QuitPlan::Stop arm calls `handle.stop(StopReason::AppQuit)`",
        "Import `StopReason` in crates/ui/src/app.rs and connection.rs from `v2ray_rs_process`"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-ui && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "l4",
      "taskIds": [
        "2.3"
      ],
      "prev": "l3",
      "sharedPkg": "crates/process/src",
      "parallel": false,
      "seam": "backend-state-logs",
      "shard": "process",
      "pkgDirs": [
        "crates/process/src"
      ],
      "pkgs": [
        "v2ray-rs-process"
      ],
      "sites": [
        {
          "task": "2.3",
          "file": "crates/process/src/state.rs",
          "symbol": "StateManager::transition",
          "anchor": "        let old = self.state.transition(target.clone())?;",
          "change": "log::info! `backend state {old:?} → {target:?}` after a successful transition"
        },
        {
          "task": "2.3",
          "file": "crates/process/src/manager.rs",
          "symbol": "handle_unexpected_exit",
          "anchor": "    async fn handle_unexpected_exit(&mut self, status: ExitStatus) {",
          "change": "after record_crash/write_exit_record log `backend exited unexpectedly (code=…); respawn crash=i/3`"
        }
      ],
      "contract": {
        "states": [
          "transition_accepted",
          "transition_rejected",
          "respawn_pending",
          "respawn_given_up",
          "exit_signal_reported",
          "no_logger_installed"
        ],
        "transitions": [
          {
            "input": "StateManager::transition with a valid target",
            "state": "transition_accepted",
            "effect": "set",
            "evidence": "anchor '        let old = self.state.transition(target.clone())?;' — one `backend state {old:?} → {target:?}` line at info after it succeeds"
          },
          {
            "input": "StateManager::transition with an invalid target",
            "state": "transition_rejected",
            "effect": "no-op",
            "evidence": "anchor '            return Err(TransitionError::Invalid {' in ProcessState::transition — the `?` returns before the log"
          },
          {
            "input": "unexpected exit with auto_restart on and crash budget left",
            "state": "respawn_pending",
            "effect": "set",
            "evidence": "anchor '    async fn handle_unexpected_exit(&mut self, status: ExitStatus) {' — one `backend exited unexpectedly (code=…); respawn crash=i/3` line after record_crash/write_exit_record"
          },
          {
            "input": "unexpected exit with auto_restart off",
            "state": "respawn_given_up",
            "effect": "forced",
            "evidence": "anchor '        if !self.auto_restart {' in handle_unexpected_exit — the exit is recorded without a respawn claim"
          },
          {
            "input": "crash_times.len() >= MAX_CRASHES",
            "state": "respawn_given_up",
            "effect": "forced",
            "evidence": "anchor '            if self.crash_times.len() >= MAX_CRASHES {'"
          },
          {
            "input": "exit by signal (code None)",
            "state": "exit_signal_reported",
            "effect": "set",
            "evidence": "anchor '    fn exit_status_field(status: Option<&ExitStatus>) -> String {' — reused so the app log and the exit record agree"
          },
          {
            "input": "state.rs unit tests transitioning directly",
            "state": "no_logger_installed",
            "effect": "no-op",
            "evidence": "codemap risk: 'StateManager::transition logging fires from tests too'"
          }
        ],
        "forbidden": [
          "logging a rejected transition",
          "logging inside `ProcessState::transition` (the pure state machine) rather than `StateManager::transition`",
          "a per-poll or per-log-line record — only state changes and respawn decisions",
          "the respawn line disagreeing with the exit record about the code/signal"
        ],
        "seeding": [
          "transition_accepted: `StateManager::new()` then `mgr.transition(ProcessState::Starting, None).unwrap()` — the shape `transition_returns_old_state` already uses",
          "transition_rejected: `mgr.transition(ProcessState::Running, None)` from Stopped, asserting `is_err()`",
          "respawn_pending: `crashing_backend(dir.path(), 1)` + `wait_for_lines`, as `failed_respawn_is_retried` does — never by calling handle_unexpected_exit directly",
          "assert the recorded facts through the state broadcast (`drain_states`) and backend.log, not through the `log` façade: no capturing logger exists in this crate"
        ],
        "budgets": [
          "MAX_CRASHES = 3 within CRASH_WINDOW = 60 s bounds the respawn lines per session",
          "one log line per accepted transition — a normal connect emits Stopped→Starting and Starting→Running only",
          "`wait_for_lines` polls up to 2 s"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST (state.rs): add `rejected_transition_changes_nothing` asserting an invalid transition returns Err and leaves `mgr.state()` unchanged (the observable guard behind 'no line for a rejected transition')",
        "TEST FIRST (manager.rs): add `crash_respawn_records_crash_count` asserting the exit records from a `crashing_backend(dir.path(), 1)` run carry `crashes_in_window=1` then a second session record follows — the same ordering the respawn line describes",
        "Log `log::info!(\"backend state {old:?} → {target:?}\")` in `StateManager::transition` after the inner transition succeeds",
        "Log `log::info!(\"backend exited unexpectedly ({}); respawn crash={}/{}\", exit_status_field(Some(&status)), self.crash_times.len(), MAX_CRASHES)` in `handle_unexpected_exit` INSIDE the restart branch — after both the `!self.auto_restart` guard and the `crash_times.len() >= MAX_CRASHES` guard, so a give-up never prints a respawn claim, and keep the give-up branches free of a respawn claim"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-process && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "l5",
      "taskIds": [
        "2.2"
      ],
      "prev": "l4",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "seam": "decision-logs",
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::Connect",
          "anchor": "            AppMsg::Connect(origin) => {",
          "change": "log::info! `connect origin=… strategy=… candidates=n profile_override=bool` after the candidate list is planned"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::start_connection",
          "anchor": "    fn start_connection(",
          "change": "alternative single choke point for the connect record (both Connect and ConnectToNode reach it); origin must be threaded in"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::schedule_auto_reconnect",
          "anchor": "    fn schedule_auto_reconnect(&mut self, sender: &ComponentSender<Self>) -> bool {",
          "change": "log `auto-reconnect scheduled attempt=i/3 delay=5s` on success and `auto-reconnect exhausted` on the refused branch"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::quit",
          "anchor": "        let plan = quit_plan(",
          "change": "log `quit requested plan=…` once the plan is known"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "candidate start/failure records",
          "anchor": "            let candidate_address = candidate.node.address().to_string();",
          "change": "log `candidate start i/n label=…` here; log `candidate failed i/n label=… reason=…` at each failures.push site (config-generation failure, start error, crash give-up) before the next candidate begins"
        }
      ],
      "contract": {
        "states": [
          "connect_recorded",
          "connect_not_recorded",
          "candidate_started",
          "candidate_failed",
          "failover_ended",
          "reconnect_scheduled",
          "reconnect_exhausted",
          "quit_recorded",
          "node_named_without_credentials"
        ],
        "transitions": [
          {
            "input": "start_connection reached with a planned candidate list",
            "state": "connect_recorded",
            "effect": "set",
            "evidence": "anchor '    fn start_connection(' — `connect origin=… strategy=… candidates=n profile_override=…`, the single choke point both Connect and ConnectToNode reach"
          },
          {
            "input": "start_connection refused by a guard (no binary, geodata missing, IPv6 gate)",
            "state": "connect_not_recorded",
            "effect": "no-op",
            "evidence": "anchor '                self.show_toast(\"No backend binary configured — check Preferences\");' — the record is written only after the guards pass"
          },
          {
            "input": "a candidate's manager is about to start",
            "state": "candidate_started",
            "effect": "set",
            "evidence": "anchor '            let candidate_address = candidate.node.address().to_string();' — `candidate start i/n label=…`"
          },
          {
            "input": "config generation failed for a candidate",
            "state": "candidate_failed",
            "effect": "set",
            "evidence": "anchor '                            &format!(\"config generation failed: {e}\"),' — same text as the CandidateFailure entry"
          },
          {
            "input": "start_with_connection returned a non-host-level Err",
            "state": "candidate_failed",
            "effect": "set",
            "evidence": "anchor '    impl CandidateFailure { fn new(label: &str, reason: &str, address: &str, port: u16) -> Self {' — reason = strip_ansi(reason)"
          },
          {
            "input": "the manager gave up after its crash budget",
            "state": "candidate_failed",
            "effect": "set",
            "evidence": "anchor '                            ProcessState::Error(msg) => {' inside the supervision loop — logged before the next candidate begins"
          },
          {
            "input": "host-level failure",
            "state": "failover_ended",
            "effect": "forced",
            "evidence": "anchor '                    if e.is_host_level() {' — the loop returns, so no later candidate line follows"
          },
          {
            "input": "schedule_auto_reconnect returns true",
            "state": "reconnect_scheduled",
            "effect": "set",
            "evidence": "anchor '        self.auto_reconnect_attempts += 1;' — `auto-reconnect scheduled attempt=i/3 delay=5s`"
          },
          {
            "input": "auto_reconnect_allowed refuses (pending_exit or attempts == MAX_AUTO_RECONNECTS)",
            "state": "reconnect_exhausted",
            "effect": "set",
            "evidence": "anchor '        if !auto_reconnect_allowed(self.pending_exit, self.auto_reconnect_attempts) {'"
          },
          {
            "input": "App::quit computes a plan",
            "state": "quit_recorded",
            "effect": "set",
            "evidence": "anchor '        let plan = quit_plan(' — `quit requested plan=…`"
          },
          {
            "input": "any decision record",
            "state": "node_named_without_credentials",
            "effect": "forced",
            "evidence": "requirement sentence: 'Records SHALL identify nodes by name and SHALL NOT include credentials'"
          }
        ],
        "forbidden": [
          "a candidate-failed line whose reason differs from the text in `CandidateFailure`'s summary (both come from the one already-stripped string)",
          "changing `terminal_failure(&failures)` or the `last_candidate_failure_reports_one_error` expected text",
          "logging the node's password, uuid, or subscription URL",
          "a connect line written for an attempt that never started",
          "installing the capturing logger outside `#[cfg(test)]`, or installing it more than once per test binary"
        ],
        "seeding": [
          "connect/auto-reconnect/quit lines: the app.rs formatter functions are free functions — unit-test them directly; app.rs tests never build an App",
          "reconnect_exhausted: call `auto_reconnect_allowed(false, MAX_AUTO_RECONNECTS)` directly, as `auto_reconnect_exhausted_after_three_attempts` does",
          "The capture is one process-wide logger per test binary: `candidate(\"203.0.113.1\"/.2/.3)` appears in at least eight other connection.rs tests, all multi_thread and concurrent under --test-threads=4. A test asserting on capture ORDER must use addresses unique to itself (198.51.100.11/.12/.13) and filter on them."
        ],
        "budgets": [
          "MAX_AUTO_RECONNECTS = 3 and AUTO_RECONNECT_DELAY = 5 s are the numbers the reconnect line prints",
          "one connect line per start_connection, one start line per candidate, at most one failed line per candidate",
          "`TestLogCapture::wait_for(needle, count)` polls at most 5 s at 50 ms",
          "log level: every decision line is `info`, i.e. visible at the default LevelFilter::Info"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST (logging.rs): add `#[cfg(test)] pub(crate) struct TestLogCapture` over a `Mutex<Vec<String>>` implementing `Log`, `pub(crate) fn install_test_capture() -> &'static TestLogCapture` installing it once via `std::sync::Once` + `log::set_boxed_logger`, with `lines_containing(&self, needle: &str) -> Vec<String>` and `wait_for(&self, needle: &str, count: usize) -> Vec<String>` (5 s cap, 50 ms poll); unit-test the capture itself with two records",
        "TEST FIRST (connection.rs): add `failover_is_traceable` — three candidates, first fails, use addresses used by no other test in the binary — 198.51.100.11/.12/.13 — and filter the capture on those; assert it holds `candidate start 1/3` with 198.51.100.11, then `candidate failed 1/3` carrying its reason, then `candidate start 2/3`, in that order",
        "TEST FIRST (app.rs): add table tests for `connect_line`, `auto_reconnect_line`, `auto_reconnect_exhausted_line` and `quit_line` asserting the exact key=value text, including `candidates=3` and `attempt=1/3 delay=5s`",
        "Add the pure formatters in app.rs beside `quit_plan`: `fn connect_line(origin: ConnectOrigin, strategy: AutoResolveStrategy, candidates: usize, profile_override: bool) -> String`, `fn auto_reconnect_line(attempt: u32, max: u32) -> String`, `fn quit_line(plan: &QuitPlan) -> String`, and an `origin_field(ConnectOrigin) -> &'static str` mapping User→user (node when a direct target is set), AutoReconnect→auto-reconnect, Restart→restart",
        "Log the connect record in `start_connection` after the guards pass, using the `origin` parameter added in the status-text seam and `uses_imported_profile` over the candidate list for `profile_override`",
        "Log `auto-reconnect scheduled` / `auto-reconnect exhausted` in `schedule_auto_reconnect`, and `quit requested plan=…` in `App::quit` after `quit_plan` returns",
        "In connection.rs log `candidate start i/n label=…` at anchor '            let candidate_address = candidate.node.address().to_string();' and `candidate failed i/n label=… reason=…` at each `failures.push(CandidateFailure::new(` site, reusing the stripped reason"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-ui && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "l6",
      "taskIds": [
        "3.1",
        "3.2"
      ],
      "prev": "l5",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "seam": "unclean-exit-paths",
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "3.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::init (signal handlers)",
          "anchor": "        let show_wizard = settings_load_error.is_none() && !settings.onboarding_complete;",
          "change": "install glib::unix_signal_add_local for SIGTERM/SIGINT/SIGHUP near the existing background-task block; each logs the signal and emits the quit path (AppMsg::TrayQuit); a second signal while pending_exit exits immediately"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::TrayQuit",
          "anchor": "            AppMsg::TrayQuit => {",
          "change": "the quit path the signal handler reuses; pending_exit gate for the force-exit on a second signal lives in App::quit"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "try_run (panic hook)",
          "anchor": "    crate::logging::init_logging(&paths);",
          "change": "install a panic hook right after, chaining std::panic::take_hook, logging error with payload message + location"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/logging.rs",
          "symbol": "install_panic_hook + test",
          "anchor": "pub fn init_logging(paths: &AppPaths) {",
          "change": "the hook and its unit test belong beside AppLogger, which already has a direct-construction test harness (log_record)"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::quit second-signal guard",
          "anchor": "    fn quit(",
          "change": "Add `fn force_exit_on_second_signal(pending_exit: bool) -> bool` and an early return in App::quit — today there is no pending_exit early return, so with the handle already taken quit_plan returns AwaitStopped and a second signal keeps waiting."
        }
      ],
      "contract": {
        "states": [
          "pending_exit=false",
          "pending_exit=true",
          "process_exited",
          "source_ids_stored",
          "panic_hook_installed",
          "panic_recorded",
          "previous_hook_ran",
          "stale_pid_present",
          "stale_pid_absent",
          "reap_skipped",
          "record_before_session"
        ],
        "transitions": [
          {
            "input": "SIGTERM|SIGINT|SIGHUP while pending_exit=false",
            "state": "pending_exit=true",
            "effect": "set",
            "evidence": "anchor '            AppMsg::TrayQuit => {' — the handler emits the same message the tray quit uses, and logs the signal"
          },
          {
            "input": "a second signal while pending_exit=true",
            "state": "process_exited",
            "effect": "forced",
            "evidence": "proposal sentence: 'a second signal while `pending_exit` is set exits immediately so a wedged stop cannot make the app unkillable'"
          },
          {
            "input": "init_logging returns during try_run",
            "state": "panic_hook_installed",
            "effect": "set",
            "evidence": "anchor '    crate::logging::init_logging(&paths);' — install_panic_hook() is called immediately after"
          }
        ],
        "forbidden": [
          "a tokio signal task instead of the GLib handler — the quit path owns GTK state and must run on the main loop",
          "a signal path that destroys the window without stopping the backend (it must reuse App::quit, so the exit record is written)",
          "a panic hook that replaces the previous hook instead of chaining it",
          "writing the unclean-previous-run record after `check_and_kill_orphaned` has removed the PID file, or from outside the lifecycle-lock block",
          "holding the short-lived RotatingFileWriter beyond the record"
        ],
        "seeding": [
          "panic record: call the pure `fn panic_record(info: &std::panic::PanicHookInfo) -> String` directly from a test that triggers it through `std::panic::catch_unwind`, and drive `AppLogger` with the existing `log_record` harness — do not install the global hook in a unit test",
          "the signal path has no unit seeding: it is manual verification (`kill -TERM` while connected, task 5.2)"
        ],
        "budgets": [
          "3 signals handled: SIGTERM, SIGINT, SIGHUP",
          "exactly 1 unclean-previous-run record per app start",
          "exactly 1 panic record per panic",
          "backend.log writer opened at DEFAULT_MAX_BYTES, the same cap the connection task uses"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST (logging.rs): add `panic_record_names_message_and_location` over the pure `panic_record`, and `install_panic_hook_chains_previous` asserting a sentinel set by a previously installed hook still fires",
        "Add `fn panic_record(info: &std::panic::PanicHookInfo) -> String` and `pub fn install_panic_hook()` (chaining `std::panic::take_hook`) to crates/ui/src/logging.rs; call `install_panic_hook()` right after `crate::logging::init_logging(&paths);`",
        "Install `glib::unix_signal_add_local` handlers for SIGTERM (15), SIGINT (2) and SIGHUP (1) in `App::init`, each logging the signal and emitting `AppMsg::TrayQuit`, returning `glib::ControlFlow::Continue`; store the SourceIds on the model; the second-signal force exit lives in `App::quit` behind the existing `pending_exit` flag",
        "TEST FIRST: add `force_exit_on_second_signal` table test — false on the first signal (pending_exit false), true on the second (pending_exit true)",
        "Add the pure `fn force_exit_on_second_signal(pending_exit: bool) -> bool` beside `quit_plan` and take the early exit path in App::quit when it returns true"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-ui && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "l7",
      "taskIds": [
        "3.3"
      ],
      "prev": "l6",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "seam": "unclean-exit-paths",
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src",
        "crates/process/src"
      ],
      "pkgs": [
        "v2ray-rs-process",
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "startup orphan reap",
          "anchor": "                    if !skip_orphans && let Err(err) = cleanup_orphaned_backend(&orphan_paths) {",
          "change": "before the reap, if paths.pid_file_path() exists, append `unclean previous run left backend pid=… killed=bool` to logs_dir()/backend.log through a short-lived RotatingFileWriter"
        },
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "cleanup_orphaned_backend",
          "anchor": "fn cleanup_orphaned_backend(paths: &AppPaths) -> std::io::Result<bool> {",
          "change": "read the PID via PidFile::read() before check_and_kill_orphaned so the record carries the pid; returns whether a live orphan was killed"
        },
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "tests (new stale-PID test)",
          "anchor": "    fn quit_idle_exits() {",
          "change": "add a temp-profile test writing a stale PID ownership record then asserting backend.log gains the unclean-previous-run record (needs AppPaths::for_profile_in like connection.rs's stub())"
        }
      ],
      "contract": {
        "states": [
          "pending_exit=false",
          "pending_exit=true",
          "process_exited",
          "source_ids_stored",
          "panic_hook_installed",
          "panic_recorded",
          "previous_hook_ran",
          "stale_pid_present",
          "stale_pid_absent",
          "reap_skipped",
          "record_before_session"
        ],
        "transitions": [
          {
            "input": "startup with paths.pid_file_path() present",
            "state": "stale_pid_present",
            "effect": "set",
            "evidence": "anchor '                    if !skip_orphans && let Err(err) = cleanup_orphaned_backend(&orphan_paths) {' — the record is appended before the reap"
          },
          {
            "input": "startup without a PID file",
            "state": "stale_pid_absent",
            "effect": "no-op",
            "evidence": "design sentence: 'The PID file is removed on every clean stop and crash exit'"
          },
          {
            "input": "settings failed to load (skip_orphans = true)",
            "state": "reap_skipped",
            "effect": "forced",
            "evidence": "anchor '            let skip_orphans = settings_load_error.is_some();' — no reap and no record"
          }
        ],
        "forbidden": [
          "a tokio signal task instead of the GLib handler — the quit path owns GTK state and must run on the main loop",
          "a signal path that destroys the window without stopping the backend (it must reuse App::quit, so the exit record is written)",
          "a panic hook that replaces the previous hook instead of chaining it",
          "writing the unclean-previous-run record after `check_and_kill_orphaned` has removed the PID file, or from outside the lifecycle-lock block",
          "holding the short-lived RotatingFileWriter beyond the record"
        ],
        "seeding": [
          "Seed the stale PID file with a LIVE but non-matching pid: `PidFile::new(paths.pid_file_path()).write(std::process::id(), Path::new(\"/bin/sh\"), &temp_config)`. `PidFile::write` builds the record through `process_start_time(pid)?`, so a dead pid returns Err(NotFound) and the test fails before asserting; serialising a PidOwnershipRecord directly is unavailable because crates/ui has no serde_json dependency. process_matches_record then fails, check_and_kill_orphaned removes the file and returns false, and the record reads `killed=false` with the pid file present before the reap."
        ],
        "budgets": [
          "3 signals handled: SIGTERM, SIGINT, SIGHUP",
          "exactly 1 unclean-previous-run record per app start",
          "exactly 1 panic record per panic",
          "backend.log writer opened at DEFAULT_MAX_BYTES, the same cap the connection task uses"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST (app.rs): add `stale_pid_file_records_unclean_previous_run` — temp profile, stale PID ownership record, call `record_unclean_previous_run(&paths)`, assert backend.log contains `unclean previous run left backend pid=` and the pid; add `no_pid_file_records_nothing`",
        "Extract `fn record_unclean_previous_run(paths: &AppPaths) -> bool` in app.rs: read the PID via `PidFile::read()` before the reap, run `cleanup_orphaned_backend`, and append `unclean previous run left backend pid=… killed=…` through a short-lived `RotatingFileWriter::open(paths.logs_dir().join(\"backend.log\"), DEFAULT_MAX_BYTES)`; call it in the startup block in place of the bare `cleanup_orphaned_backend` call"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-process && make test-ui && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "l8",
      "taskIds": [
        "4.1"
      ],
      "prev": "l7",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "seam": "status-text",
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "4.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "status_primary (new pure fn) + update_status_labels",
          "anchor": "            (ProcessState::Starting, _) => (\"Connecting…\".to_string(), \"Resolving nodes\".into()),",
          "change": "replace the Starting arm with status_primary(state, prev_state, origin, attempt) → \"Restarting after crash\" / \"Reconnecting (n/3)\" / \"Connecting…\""
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::apply_state",
          "anchor": "        let from = self.process_state.clone();",
          "change": "the previous state needed by status_primary is already captured here; store it (or pass it) for update_status_labels"
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "App struct fields",
          "anchor": "    auto_reconnect_attempts: u32,",
          "change": "add the current connection's ConnectOrigin (and reuse auto_reconnect_attempts as the attempt number); set it where start_connection is called"
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "tests (status_primary table)",
          "anchor": "    fn toggle_disabled_while_stopping() {",
          "change": "add unit tests for each status_primary case beside the existing pure-fn tests"
        }
      ],
      "contract": {
        "states": [
          "origin=User",
          "origin=AutoReconnect",
          "origin=Restart",
          "prev=Running",
          "prev=Stopped",
          "text=Connected",
          "text=RestartingAfterCrash",
          "text=Reconnecting",
          "text=Connecting",
          "text=unchanged",
          "status_not_updated",
          "attempt_preserved"
        ],
        "transitions": [
          {
            "input": "state=Running with metadata",
            "state": "text=Connected",
            "effect": "no-op",
            "evidence": "anchor '                (\"Connected\".to_string(), details)' — details string unchanged"
          },
          {
            "input": "state=Starting with prev=Running",
            "state": "text=RestartingAfterCrash",
            "effect": "set",
            "evidence": "anchor '        let from = self.process_state.clone();' in apply_state — the previous state is already captured there"
          },
          {
            "input": "state=Starting, prev != Running, origin=AutoReconnect",
            "state": "text=Reconnecting",
            "effect": "set",
            "evidence": "anchor '        self.auto_reconnect_attempts += 1;' in schedule_auto_reconnect — n is the attempt, denominator MAX_AUTO_RECONNECTS"
          },
          {
            "input": "state=Starting, prev != Running, origin=User or origin=Restart",
            "state": "text=Connecting",
            "effect": "no-op",
            "evidence": "anchor '            (ProcessState::Starting, _) => (\"Connecting…\".to_string(), \"Resolving nodes\".into()),'"
          },
          {
            "input": "start_connection called (new generation)",
            "state": "prev=Stopped",
            "effect": "forced",
            "evidence": "anchor '        self.apply_state(&ProcessState::Starting);' in start_connection — a fresh connect is not a respawn; origin is set from its new parameter"
          },
          {
            "input": "state=Stopping, Stopped or Error",
            "state": "text=unchanged",
            "effect": "no-op",
            "evidence": "anchor '            (ProcessState::Stopping, _) => {'"
          },
          {
            "input": "a superseded generation reports Starting",
            "state": "status_not_updated",
            "effect": "no-op",
            "evidence": "anchor '                if !is_current_generation(generation, self.connection_generation) {'"
          },
          {
            "input": "connect with origin=AutoReconnect",
            "state": "attempt_preserved",
            "effect": "no-op",
            "evidence": "anchor 'fn cancels_auto_reconnect(origin: ConnectOrigin) -> bool {' — `origin != ConnectOrigin::AutoReconnect`, so the counter is not reset"
          }
        ],
        "forbidden": [
          "two pure status functions over the same widget pair — one `status_texts(&StatusView) -> (String, String)` only (this supersedes tasks.md's `status_primary` name, per the sprint's one-status-function decision)",
          "'Restarting after crash' shown for a fresh connect or for a state reported by a superseded generation",
          "'Reconnecting (n/3)' with n read from anything but the single attempt accessor",
          "reading the health/dns fields in a non-Running arm (they belong to the sibling change and `App` clears them outside Running)"
        ],
        "seeding": [
          "`status_texts` is a free function over `StatusView` — build the struct literally in a table test beside `toggle_disabled_while_stopping`; app.rs tests never construct an App or a GTK widget",
          "connection metadata for the Running arms: the existing `snapshot` / `session_target_node` fixtures",
          "prev_state is reached only through `apply_state`'s captured `from` in production; a test sets the field on `StatusView` directly"
        ],
        "budgets": [
          "MAX_AUTO_RECONNECTS = 3 is the denominator printed in 'Reconnecting (n/3)'",
          "n ranges over 1..=3 — attempt 0 with origin AutoReconnect must still render 'Connecting…'"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST: add `status_texts_covers_every_starting_case` — a table over StatusView asserting Restarting after crash / Reconnecting (1/3) / Connecting… and that the Running, Stopping, Stopped and Error arms are byte-identical to today's strings",
        "Add `struct StatusView { state: ProcessState, prev_state: ProcessState, meta: Option<ConnectionMetadata>, origin: ConnectOrigin, attempt: u32 }` and `fn status_texts(view: &StatusView) -> (String, String)` in app.rs, holding today's `update_status_labels` match plus the two new Starting arms (the sibling change adds `health` and `dns_failing` fields and the Running arms that read them)",
        "Add fields `status_prev: ProcessState` and `connection_origin: ConnectOrigin` to `App`; set `status_prev` from the `from` already captured in `apply_state`, and force it to `ProcessState::Stopped` in `start_connection` before its `apply_state(&ProcessState::Starting)`",
        "Add an `origin: ConnectOrigin` parameter to `fn start_connection(` and thread it from both call sites (the AppMsg::Connect arm and the AppMsg::ConnectToNode arm), storing it in `connection_origin`",
        "Rewrite `update_status_labels` to build a `StatusView` from the model and call `status_texts`"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-ui && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "l9",
      "taskIds": [
        "5.1",
        "5.2"
      ],
      "prev": "l8",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "seam": "verification",
      "shard": "",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-core",
        "v2ray-rs-process",
        "v2ray-rs-ui"
      ],
      "sites": [],
      "contract": {
        "states": [
          "floor_green",
          "live_verified"
        ],
        "transitions": [
          {
            "input": "make fmt && make clippy && make test TEST_TIMEOUT=10m",
            "state": "floor_green",
            "effect": "set",
            "evidence": "Makefile anchors 'fmt:', 'clippy:' and 'test:' — TEST := timeout $(TEST_TIMEOUT) $(CARGO) test, TEST_ARGS := -- --test-threads=$(TEST_THREADS)"
          },
          {
            "input": "live: connect, apply-with-restart, node switch, disconnect, quit",
            "state": "live_verified",
            "effect": "set",
            "evidence": "spec scenario 'Stop reasons are distinguished' — the four exit records read apply-restart, node-switch, user-stop, app-quit"
          },
          {
            "input": "live: kill -TERM while connected",
            "state": "live_verified",
            "effect": "set",
            "evidence": "tasks.md 3.1: 'verified live' — app log records the signal, backend.log has reason=app-quit, TUN device gone"
          }
        ],
        "forbidden": [
          "a bare `cargo test` without a timeout and a thread cap (the repo's own test-limits rule)",
          "claiming 5.2 done from unit tests — it needs a real backend and a real TUN device"
        ],
        "seeding": [
          "floor: run from the repo root against a clean working tree",
          "live: a real xray or sing-box binary with TUN granted; not reachable in CI"
        ],
        "budgets": [
          "workspace tests: timeout 10m, --test-threads=4",
          "per-crate tests: timeout 5m, --test-threads=4"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "Run the full floor and paste the failing output verbatim if anything is red",
        "MANUAL: execute the task 5.2 live pass and record the observed exit reasons and app-log lines"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "TEST_TIMEOUT=10m make test && make lint",
      "coder": "rust-coder"
    }
  ],
  "seams": [
    {
      "id": "exit-records",
      "tasks": [
        "1.1",
        "1.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no red-stage agent in this workflow, so no sealed test author runs before the coder. NO-TESTER-WAIVER: same reason — the coder writes the tests first, inside crates/process/src/manager.rs's existing `mod tests`. Seam: `StopReason` + `shutdown_with(reason)` in v2ray-rs-process; `write_exit_record` gains `reason=` and `last_error=`; launch-failure stops carry `start-failed`, unrequested exits after readiness carry the literal `crash`. Test files the coder may change: crates/process/src/manager.rs (`mod tests` only) — no new files.",
      "contract": {
        "states": [
          "stop_reason=UserStop",
          "stop_reason=NodeSwitch",
          "stop_reason=ApplyRestart",
          "stop_reason=AppQuit",
          "stop_reason=StartFailed",
          "exit_record_requested",
          "exit_record_crash",
          "exit_record_absent",
          "last_error=line",
          "last_error=none",
          "error_text_unchanged",
          "exit_record_start_failed"
        ],
        "transitions": [
          {
            "input": "ProcessManager::new(..) constructed",
            "state": "stop_reason=UserStop",
            "effect": "set",
            "evidence": "anchor '            cached_version: None,' in ProcessManager::new — the new `stop_reason` field is initialized beside it"
          },
          {
            "input": "shutdown() with no explicit reason",
            "state": "stop_reason=UserStop",
            "effect": "set",
            "evidence": "anchor '    pub async fn shutdown(&mut self) {' — shutdown() delegates to shutdown_with(StopReason::UserStop)"
          },
          {
            "input": "shutdown_with(StopReason::NodeSwitch)",
            "state": "stop_reason=NodeSwitch",
            "effect": "set",
            "evidence": "anchor '    pub async fn shutdown(&mut self) {' — shutdown_with stores the reason before running today's body"
          },
          {
            "input": "shutdown_with(StopReason::ApplyRestart)",
            "state": "stop_reason=ApplyRestart",
            "effect": "set",
            "evidence": "anchor '    pub async fn shutdown(&mut self) {'"
          },
          {
            "input": "shutdown_with(StopReason::AppQuit)",
            "state": "stop_reason=AppQuit",
            "effect": "set",
            "evidence": "anchor '    pub async fn shutdown(&mut self) {'"
          },
          {
            "input": "stop() called without a preceding shutdown_with",
            "state": "stop_reason=UserStop",
            "effect": "no-op",
            "evidence": "anchor '    pub async fn stop(&mut self) -> Result<(), ProcessError> {' — stop takes no reason parameter and leaves the stored reason alone"
          },
          {
            "input": "TUN device did not appear within tun::DEVICE_TIMEOUT during launch",
            "state": "stop_reason=StartFailed",
            "effect": "forced",
            "evidence": "anchor '                return Err(ProcessError::TunDeviceTimeout(rt.iface.clone()));' — set before the graceful_stop() that precedes it"
          },
          {
            "input": "xray-up helper returned Err during launch",
            "state": "stop_reason=StartFailed",
            "effect": "forced",
            "evidence": "anchor '                return Err(ProcessError::TunHelper(e));' — set before the preceding graceful_stop()"
          },
          {
            "input": "graceful_stop completes (requested stop)",
            "state": "exit_record_requested",
            "effect": "set",
            "evidence": "anchor '        self.write_exit_record(true, status.as_ref());' at the tail of graceful_stop — prints `requested=true reason=<stop_reason.as_str()>`"
          },
          {
            "input": "child exited while state was Running",
            "state": "exit_record_crash",
            "effect": "forced",
            "evidence": "anchor '        self.write_exit_record(false, Some(&status));' in handle_unexpected_exit — prints `requested=false reason=crash`"
          },
          {
            "input": "child.wait() returned Err",
            "state": "exit_record_crash",
            "effect": "forced",
            "evidence": "anchor '                self.write_exit_record(false, None);' in wait_and_handle_exit — prints `requested=false reason=crash code=none`"
          },
          {
            "input": "log_writer is None",
            "state": "exit_record_absent",
            "effect": "no-op",
            "evidence": "anchor '    fn write_exit_record(&self, requested: bool, status: Option<&ExitStatus>) {' — its first statement returns when log_writer.is_none()"
          },
          {
            "input": "buffered output holds a [Warning] line followed by access lines",
            "state": "last_error=line",
            "effect": "set",
            "evidence": "anchor '    fn last_output_line(&self) -> Option<String> {' — last_error_line scans the same buffer.last_n(50); last_output stays the access line"
          },
          {
            "input": "no matching warning/error token in the last 50 lines",
            "state": "last_error=none",
            "effect": "set",
            "evidence": "requirement sentence: 'the last warning, error, or fatal line among the recent output (or `none`)'"
          },
          {
            "input": "matching line wrapped in CSI colour codes",
            "state": "last_error=line",
            "effect": "set",
            "evidence": "tasks.md 1.2: '(`[Warning]`, `[Error]`, `WARN`, `ERROR`, `FATAL`, `panic`, ANSI-tolerant)'"
          },
          {
            "input": "crash exit with a last stderr line",
            "state": "error_text_unchanged",
            "effect": "no-op",
            "evidence": "anchor '        if let Some(reason) = self.last_output_line() {' in handle_unexpected_exit — last_output_line is not modified"
          },
          {
            "input": "child exited before ready (readiness path)",
            "state": "exit_record_start_failed",
            "effect": "forced",
            "evidence": "tasks.md 1.1: 'a backend that exits before ready records requested=false reason=start-failed crashes_in_window=0 from the readiness path, never through graceful_stop'"
          }
        ],
        "forbidden": [
          "`reason=crash` together with `requested=true` — a requested stop is never a crash",
          "`reason=user-stop` on any launch-failure path (TunDeviceTimeout, TunHelper)",
          "write_exit_record accepting a `StopReason` for the unrequested path — `crash` is not a StopReason variant and must come from the single `CRASH_REASON` constant",
          "changing `last_output_line`'s selection rule or the crash `Error` message text",
          "more than one exit record per child exit"
        ],
        "seeding": [
          "requested stop with a given reason: `mgr.shutdown_with(StopReason::NodeSwitch).await` (or the matching variant) after a successful `mgr.start().await`",
          "crash record: `manager_for(&dir, \"...exit 3\")` + `mgr.set_auto_restart(false)` + `mgr.wait_and_handle_exit().await`, exactly as `exit_record_marks_unrequested_crash` does",
          "last_error: stub body prints a `[Warning] ...` line, then access-shaped stdout lines, then exits — read the record via `wait_for_lines(&dir.path().join(\"backend.log\"), 2)`",
          "records are read from the file with `read_lines` / `wait_for_lines`, never from the broadcast channel",
          "Seed `exit_record_marks_start_failed_on_tun_device_timeout` with a stub helper whose `xray-up` exits 1 (ProcessError::TunHelper), which is instant. Do NOT seed it with `xray_on_lo`: its iface is `lo`, which always exists, so wait_for_device returns immediately and the timeout never fires. The nonexistent-iface variant works but costs a fixed tun::DEVICE_TIMEOUT of 10 s that no manager field overrides."
        ],
        "budgets": [
          "last_error_line scans exactly the last 50 buffered lines (`buffer.last_n(50)`), the same window as last_output_line",
          "reason and last_error text pass through `truncate_reason`: REASON_MAX_CHARS = 200 chars, newline-scrubbed",
          "`wait_for_lines` polls up to 2 s for the expected line count",
          "per-test wall clock: the seam's verify command caps the whole crate at 5 m",
          "tun::DEVICE_TIMEOUT = 10 s, fixed and not per-manager overridable — the reason the helper-failure seeding is preferred"
        ]
      },
      "codeTasks": [
        "TEST FIRST: add `exit_before_ready_records_start_failed` — FATAL-then-exit stub with a ready probe (the harness confirm-backend-ready-before-running adds), assert the exit record carries `requested=false reason=start-failed crashes_in_window=0` and that no `reason=user-stop` record is written",
        "TEST FIRST: extend `exit_record_marks_requested_stop` to assert `reason=user-stop`, and add `exit_record_reason_per_stop_reason` driving shutdown_with for NodeSwitch/ApplyRestart/AppQuit and asserting `reason=node-switch|apply-restart|app-quit`",
        "TEST FIRST: add `exit_record_marks_start_failed_on_tun_device_timeout` using `stub_helper` with an xray-up that exits 1 (ProcessError::TunHelper — instant). Do NOT use `xray_on_lo`: its iface is `lo`, which always exists, so wait_for_device returns immediately and the timeout never fires; the nonexistent-iface variant costs a fixed tun::DEVICE_TIMEOUT of 10 s that no manager field overrides. Assert `requested=true reason=start-failed`",
        "TEST FIRST: extend `exit_record_marks_unrequested_crash` to assert `requested=false reason=crash`",
        "TEST FIRST: add `last_error_survives_access_noise` — stub prints `[Warning] cert about to expire`, then two access-shaped stdout lines, then exits; assert `last_error=[Warning] cert about to expire` and `last_output=<final access line>`, and that the ProcessState::Error text still ends with the final buffered line (unchanged behavior)",
        "TEST FIRST: add `last_error_is_none_without_warnings` and `last_error_matches_through_ansi` (stub prints `\\033[33m[Warning]\\033[0m tls retry`)",
        "Add `pub enum StopReason { UserStop, NodeSwitch, ApplyRestart, AppQuit, StartFailed }` (Debug, Clone, Copy, PartialEq, Eq) with `pub fn as_str(&self) -> &'static str` yielding user-stop|node-switch|apply-restart|app-quit|start-failed and `impl std::fmt::Display` delegating to as_str, beside `const REASON_MAX_CHARS: usize = 200;` in crates/process/src/manager.rs; add `const CRASH_REASON: &str = \"crash\";` there too",
        "Add field `stop_reason: StopReason` to `ProcessManager`, initialized to `StopReason::UserStop` next to `cached_version: None,` in `ProcessManager::new`",
        "Add `pub async fn shutdown_with(&mut self, reason: StopReason)` holding today's `shutdown` body preceded by `self.stop_reason = reason;`; `shutdown` becomes `self.shutdown_with(StopReason::UserStop).await`",
        "Set `self.stop_reason = StopReason::StartFailed;` immediately before the `graceful_stop()` calls guarding `ProcessError::TunDeviceTimeout` and `ProcessError::TunHelper` in `launch`",
        "Change the signature to `fn write_exit_record(&self, requested: bool, reason: &str, status: Option<&ExitStatus>)` and emit `requested={requested} reason={reason} {status} crashes_in_window={} last_output={last} last_error={err}`; graceful_stop passes `self.stop_reason.as_str()`, both unrequested sites pass `CRASH_REASON`",
        "Add `fn last_error_line(&self) -> Option<String>` beside `last_output_line`, scanning `buffer.last_n(50)` in reverse for a line whose ANSI-stripped content contains any of `[Warning]`, `[Error]`, `WARN`, `ERROR`, `FATAL`, `panic`, returning it through `truncate_reason`; add the private helper `fn without_ansi(line: &str) -> String` next to `truncate_reason`",
        "Re-export `StopReason` from crates/process/src/lib.rs at anchor 'pub use manager::{ProcessError, ProcessManager};'",
        "Emit the readiness-failure exit record from the readiness path itself with `write_exit_record(false, StopReason::StartFailed.as_str(), Some(&status))`; do NOT let it fall through to graceful_stop, whose stored `stop_reason` defaults to UserStop and would emit `requested=true reason=user-stop` — banned by this seam's own forbidden list",
        "Reset `self.stop_reason = StopReason::UserStop;` after `write_exit_record` so a launch failure's StartFailed does not leak into a later stop on the same manager"
      ]
    },
    {
      "id": "session-fields",
      "tasks": [
        "1.3"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no red-stage agent. NO-TESTER-WAIVER: same — the coder writes these tests first. Seam: `ProcessManager::with_session_fields(String)` appends caller-supplied fields to the session record; the connection task builds `hijack capture_dns strict nodes_pinned profile`, and core gains the `uses_imported_profile` predicate the `profile=` field needs. Test files the coder may change: crates/process/src/manager.rs (`mod tests`), crates/ui/src/connection.rs (`mod tests`), crates/core/src/models/imported_profile.rs (`mod tests`) — no new files.",
      "contract": {
        "states": [
          "session_fields=None",
          "session_fields=Some(text)",
          "session_record_single_line",
          "session_record_repeated",
          "tun_runtime=None",
          "capture_dns=true",
          "strict=false",
          "profile=imported",
          "profile=app"
        ],
        "transitions": [
          {
            "input": "manager built without with_session_fields",
            "state": "session_fields=None",
            "effect": "no-op",
            "evidence": "anchor '            &format!(\"backend={backend} version={version} node={node} tun={tun}\"),' in write_session_record — the line ends at tun="
          },
          {
            "input": "with_session_fields(text) chained in the builder",
            "state": "session_fields=Some(text)",
            "effect": "set",
            "evidence": "anchor '    pub fn with_log_file(mut self, writer: Option<Arc<RotatingFileWriter>>) -> Self {' — the builder shape with_session_fields copies"
          },
          {
            "input": "field text containing \\n or \\r",
            "state": "session_record_single_line",
            "effect": "forced",
            "evidence": "anchor '    fn truncate_reason(line: &str) -> String {' comment 'Records are single-line'"
          },
          {
            "input": "respawn after a crash",
            "state": "session_record_repeated",
            "effect": "set",
            "evidence": "anchor '        self.write_session_record().await;' — the first statement of launch, which respawn also calls"
          },
          {
            "input": "TUN disabled or backend v2ray",
            "state": "tun_runtime=None",
            "effect": "forced",
            "evidence": "anchor '    if !settings.tun.enabled {' in build_tun_runtime — fields read hijack=off capture_dns=false strict=false"
          },
          {
            "input": "xray + tun.enabled + dns_hijack Hijack + every node hostname pinned",
            "state": "capture_dns=true",
            "effect": "set",
            "evidence": "anchor '        capture_dns: backend == BackendType::Xray' in build_tun_runtime"
          },
          {
            "input": "drop_strict_route decided for the connection",
            "state": "strict=false",
            "effect": "forced",
            "evidence": "anchor '            if drop_strict_route {' inside the candidate loop — effective_settings.tun.strict_route is cleared before build_tun_runtime"
          },
          {
            "input": "candidate's subscription has use_imported_profile and an imported_profile",
            "state": "profile=imported",
            "effect": "set",
            "evidence": "anchor '        && sub.use_imported_profile' in resolve_effective_config"
          },
          {
            "input": "manual node, or subscription without an imported profile",
            "state": "profile=app",
            "effect": "set",
            "evidence": "anchor '    (global_rules.to_vec(), settings.clone())' — the fallback arm of resolve_effective_config"
          }
        ],
        "forbidden": [
          "computing `profile=` by re-deriving the imported-profile condition inline in connection.rs — it must call the single core predicate",
          "reading `settings.tun.*` rather than `effective_settings.tun.*` for the session fields (the imported profile and the strict-route drop change them)",
          "the session fields naming a node's credentials, address, or port — only the five keys",
          "more than one session record per launch"
        ],
        "seeding": [
          "session_fields=Some: build through the production builder chain in connection.rs, or in process tests `manager_for(..).with_log_file(Some(backend_log(dir.path()))).with_session_fields(\"hijack=hijack capture_dns=true\".into())`",
          "tun_runtime=Some(rt) in a ui test: `tun_settings()` (or `strict_route_settings()`) passed to `connect(&stub, ..)` with the `capless_probe` configure hook",
          "profile=imported: a `Subscription` fixture with `use_imported_profile = true` and `imported_profile = Some(..)`, reached through `resolve_effective_config` — never by setting the field on the record",
          "read the session line from `stub.paths.logs_dir().join(\"backend.log\")` after the first Running state, as `live_connect_writes_backend_diagnostics` does"
        ],
        "budgets": [
          "exactly 1 session record per launch (`contents.matches(\" session \").count() == 1` for a single-candidate connect)",
          "session field text truncated at REASON_MAX_CHARS = 200",
          "ui connect tests wait for a state at most 20 s (`next_state`)"
        ]
      },
      "codeTasks": [
        "TEST FIRST (core): add `uses_imported_profile_matches_resolve_effective_config` in crates/core/src/models/imported_profile.rs — true for a subscription node whose subscription has use_imported_profile + imported_profile, false for a manual node, false when use_imported_profile is off",
        "TEST FIRST (process): add `session_record_appends_caller_fields` — `with_session_fields(\"hijack=hijack capture_dns=true strict=false nodes_pinned=true profile=app\".into())`, assert the single session line contains `tun=off hijack=hijack` and the whole suffix; add `session_fields_cannot_forge_a_second_line` with an embedded \\n",
        "TEST FIRST (ui): update `live_connect_writes_backend_diagnostics` so the existing assertion absorbs the suffix (assert `contents.contains(\"backend=sing-box version=1.13.0 node=203.0.113.1 tun=off\")` still holds and the same line ends with `hijack=off capture_dns=false strict=false nodes_pinned=true profile=app`)",
        "TEST FIRST (ui): add `xray_tun_session_record_carries_dns_decisions` — stub xray backend, `tun_settings()` with `capless_probe`, assert the session line contains `tun=on hijack=hijack capture_dns=true strict=false nodes_pinned=true profile=app`",
        "Add `pub fn uses_imported_profile(node_ref: &ConnectionNodeRef, subscriptions: &[Subscription]) -> bool` in crates/core/src/models/imported_profile.rs holding the exact condition of resolve_effective_config's first arm; make resolve_effective_config's arm read through it so the two cannot drift; extend the re-export at anchor 'pub use imported_profile::{ImportedProfile, resolve_effective_config};'",
        "Add field `session_fields: Option<String>` to `ProcessManager` and builder `pub fn with_session_fields(mut self, fields: String) -> Self` following the with_log_file shape",
        "In `write_session_record`, append `truncate_reason(&fields)` after `tun={tun}` when session_fields is Some",
        "In crates/ui/src/connection.rs add `fn hijack_field(mode: DnsHijackMode) -> &'static str` (hijack|native|disabled) and chain `.with_session_fields(format!(\"hijack={} capture_dns={} strict={} nodes_pinned={} profile={}\", ..))` onto the builder at anchor '            let mut mgr = configure(' — values from the built `tun` runtime (None → `off`/`false`/`false`), `pinned`, and `uses_imported_profile(&candidate.node_ref, &subscriptions)` → `imported`|`app`"
      ]
    },
    {
      "id": "stop-reason-plumbing",
      "tasks": [
        "2.1"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no red-stage agent. NO-TESTER-WAIVER: same. Seam: `ConnectionCmd::Stop(StopReason)` carried from the app's Disconnect/quit decision down to `ProcessManager::shutdown_with`, with the pure mapping `stop_reason_for` unit-tested in app.rs. Test files the coder may change: crates/ui/src/app.rs (`mod tests`), crates/ui/src/connection.rs (`mod tests`).",
      "contract": {
        "states": [
          "reason=UserStop",
          "reason=NodeSwitch",
          "reason=ApplyRestart",
          "reason=AppQuit",
          "reason=StartFailed",
          "reason=carried",
          "no_shutdown_call",
          "terminal_state_unchanged"
        ],
        "transitions": [
          {
            "input": "stop_reason_for(pending_exit=true, ..)",
            "state": "reason=AppQuit",
            "effect": "forced",
            "evidence": "anchor '    fn quit(&mut self, sender: &ComponentSender<Self>) {' — QuitPlan::Stop sets pending_exit then stops the handle"
          },
          {
            "input": "stop_reason_for(false, reconnect_pending=true, direct_target=None)",
            "state": "reason=ApplyRestart",
            "effect": "set",
            "evidence": "anchor '                self.reconnect_pending = true;' in the AppMsg::ApplyAndRestart arm, dispatched straight into AppMsg::Disconnect"
          },
          {
            "input": "stop_reason_for(false, false, direct_target=Some)",
            "state": "reason=NodeSwitch",
            "effect": "set",
            "evidence": "anchor '                    self.pending_direct_target = Some(target);' in the AppMsg::ConnectToNode arm, which sets reconnect_pending = false first"
          },
          {
            "input": "stop_reason_for(false, false, None)",
            "state": "reason=UserStop",
            "effect": "set",
            "evidence": "anchor '            AppMsg::Disconnect => {' — the plain Disconnect click"
          },
          {
            "input": "App::quit with QuitPlan::Stop",
            "state": "reason=AppQuit",
            "effect": "forced",
            "evidence": "anchor '                    handle.stop();' inside QuitPlan::Stop — becomes handle.stop(StopReason::AppQuit)"
          },
          {
            "input": "Stop received while the task is still queued behind the previous teardown",
            "state": "no_shutdown_call",
            "effect": "no-op",
            "evidence": "anchor '        // A Stop that arrived while we were queued behind the previous' — nothing has started yet; Stopped is reported"
          },
          {
            "input": "Stop received at the candidate-loop head",
            "state": "reason=carried",
            "effect": "set",
            "evidence": "anchor \"'candidates: for candidate in candidates {\" — passed to parked.shutdown_with(reason)"
          },
          {
            "input": "Stop received during start_with_connection",
            "state": "reason=carried",
            "effect": "set",
            "evidence": "anchor '            let started = tokio::select! {' — passed to mgr.shutdown_with and parked.shutdown_with"
          },
          {
            "input": "Stop queued behind a successful start",
            "state": "reason=carried",
            "effect": "set",
            "evidence": "anchor '                    // A Disconnect clicked while the start was in flight sits'"
          },
          {
            "input": "Stop received in the supervision loop",
            "state": "reason=carried",
            "effect": "set",
            "evidence": "anchor '                    Some(ConnectionCmd::Stop) = cmd_rx.recv() => {'"
          },
          {
            "input": "host-level start failure teardown",
            "state": "reason=StartFailed",
            "effect": "forced",
            "evidence": "anchor '                        if grant_fixable(&e) {' — the shutdown() calls above it"
          },
          {
            "input": "any Stop path",
            "state": "terminal_state_unchanged",
            "effect": "no-op",
            "evidence": "anchor '    fn relays(state: &ProcessState) -> bool {' — terminal states stay with the supervising loop"
          }
        ],
        "forbidden": [
          "`reason=UserStop` reaching the manager on a host-level start failure or a queued-behind-start teardown",
          "inferring the reason inside connection.rs from state rather than carrying it in the command",
          "a `handle.stop()` call site left without an explicit reason (the parameter is mandatory, no Default)",
          "changing which terminal state each Stop path reports"
        ],
        "seeding": [
          "stop_reason_for is a free function: call it directly with the three booleans/Option in a table test, next to `reconnect_pending_consumed_on_stop_or_error`",
          "connection-side reasons: drive `handle.stop(StopReason::NodeSwitch)` in a `connect(&stub, ..)` test and read `reason=node-switch` back from `stub.paths.logs_dir().join(\"backend.log\")`",
          "app.rs tests never construct an App or a GTK widget — the mapping must therefore stay a free function over plain inputs"
        ],
        "budgets": [
          "exactly 6 `ConnectionCmd::Stop` match sites in connection.rs after the change (queued-before-start, loop head, select! during start, queued-after-start, supervision loop, plus the host-level teardown that constructs StartFailed itself)",
          "cmd channel capacity stays 4 (`mpsc::channel::<ConnectionCmd>(4)`)",
          "ui connect tests wait at most 20 s per state (`next_state`)"
        ]
      },
      "codeTasks": [
        "TEST FIRST (app.rs): add `stop_reason_for_maps_every_input` — a table over (pending_exit, reconnect_pending, direct_target.is_some()) asserting AppQuit / ApplyRestart / NodeSwitch / UserStop with the precedence above",
        "TEST FIRST (connection.rs): add `stop_reason_reaches_the_exit_record` — connect a stub, `handle.stop(StopReason::NodeSwitch)`, wait for Stopped, assert backend.log's exit record contains `reason=node-switch`",
        "Change `enum ConnectionCmd { Stop }` to `Stop(StopReason)` and `ConnectionHandle::stop(&self, reason: StopReason)` at anchor '        let _ = self.cmd_tx.try_send(ConnectionCmd::Stop);'",
        "Update all six Stop match sites per the transition table, passing the reason to `shutdown_with`; the host-level teardown at anchor '                        if grant_fixable(&e) {' uses `StopReason::StartFailed`",
        "Add `fn stop_reason_for(pending_exit: bool, reconnect_pending: bool, direct_target: bool) -> StopReason` in app.rs beside `disconnect_plan`, and call it in the DisconnectPlan::Stop arm at anchor '                            handle.stop();'",
        "`App::quit`'s QuitPlan::Stop arm calls `handle.stop(StopReason::AppQuit)`",
        "Import `StopReason` in crates/ui/src/app.rs and connection.rs from `v2ray_rs_process`"
      ]
    },
    {
      "id": "backend-state-logs",
      "tasks": [
        "2.3"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no red-stage agent. NO-TESTER-WAIVER: same. Seam: `StateManager::transition` logs every accepted transition, and `handle_unexpected_exit` logs the crash-respawn decision with its crash count. Test files the coder may change: crates/process/src/state.rs (`mod tests`), crates/process/src/manager.rs (`mod tests`).",
      "contract": {
        "states": [
          "transition_accepted",
          "transition_rejected",
          "respawn_pending",
          "respawn_given_up",
          "exit_signal_reported",
          "no_logger_installed"
        ],
        "transitions": [
          {
            "input": "StateManager::transition with a valid target",
            "state": "transition_accepted",
            "effect": "set",
            "evidence": "anchor '        let old = self.state.transition(target.clone())?;' — one `backend state {old:?} → {target:?}` line at info after it succeeds"
          },
          {
            "input": "StateManager::transition with an invalid target",
            "state": "transition_rejected",
            "effect": "no-op",
            "evidence": "anchor '            return Err(TransitionError::Invalid {' in ProcessState::transition — the `?` returns before the log"
          },
          {
            "input": "unexpected exit with auto_restart on and crash budget left",
            "state": "respawn_pending",
            "effect": "set",
            "evidence": "anchor '    async fn handle_unexpected_exit(&mut self, status: ExitStatus) {' — one `backend exited unexpectedly (code=…); respawn crash=i/3` line after record_crash/write_exit_record"
          },
          {
            "input": "unexpected exit with auto_restart off",
            "state": "respawn_given_up",
            "effect": "forced",
            "evidence": "anchor '        if !self.auto_restart {' in handle_unexpected_exit — the exit is recorded without a respawn claim"
          },
          {
            "input": "crash_times.len() >= MAX_CRASHES",
            "state": "respawn_given_up",
            "effect": "forced",
            "evidence": "anchor '            if self.crash_times.len() >= MAX_CRASHES {'"
          },
          {
            "input": "exit by signal (code None)",
            "state": "exit_signal_reported",
            "effect": "set",
            "evidence": "anchor '    fn exit_status_field(status: Option<&ExitStatus>) -> String {' — reused so the app log and the exit record agree"
          },
          {
            "input": "state.rs unit tests transitioning directly",
            "state": "no_logger_installed",
            "effect": "no-op",
            "evidence": "codemap risk: 'StateManager::transition logging fires from tests too'"
          }
        ],
        "forbidden": [
          "logging a rejected transition",
          "logging inside `ProcessState::transition` (the pure state machine) rather than `StateManager::transition`",
          "a per-poll or per-log-line record — only state changes and respawn decisions",
          "the respawn line disagreeing with the exit record about the code/signal"
        ],
        "seeding": [
          "transition_accepted: `StateManager::new()` then `mgr.transition(ProcessState::Starting, None).unwrap()` — the shape `transition_returns_old_state` already uses",
          "transition_rejected: `mgr.transition(ProcessState::Running, None)` from Stopped, asserting `is_err()`",
          "respawn_pending: `crashing_backend(dir.path(), 1)` + `wait_for_lines`, as `failed_respawn_is_retried` does — never by calling handle_unexpected_exit directly",
          "assert the recorded facts through the state broadcast (`drain_states`) and backend.log, not through the `log` façade: no capturing logger exists in this crate"
        ],
        "budgets": [
          "MAX_CRASHES = 3 within CRASH_WINDOW = 60 s bounds the respawn lines per session",
          "one log line per accepted transition — a normal connect emits Stopped→Starting and Starting→Running only",
          "`wait_for_lines` polls up to 2 s"
        ]
      },
      "codeTasks": [
        "TEST FIRST (state.rs): add `rejected_transition_changes_nothing` asserting an invalid transition returns Err and leaves `mgr.state()` unchanged (the observable guard behind 'no line for a rejected transition')",
        "TEST FIRST (manager.rs): add `crash_respawn_records_crash_count` asserting the exit records from a `crashing_backend(dir.path(), 1)` run carry `crashes_in_window=1` then a second session record follows — the same ordering the respawn line describes",
        "Log `log::info!(\"backend state {old:?} → {target:?}\")` in `StateManager::transition` after the inner transition succeeds",
        "Log `log::info!(\"backend exited unexpectedly ({}); respawn crash={}/{}\", exit_status_field(Some(&status)), self.crash_times.len(), MAX_CRASHES)` in `handle_unexpected_exit` INSIDE the restart branch — after both the `!self.auto_restart` guard and the `crash_times.len() >= MAX_CRASHES` guard, so a give-up never prints a respawn claim, and keep the give-up branches free of a respawn claim"
      ]
    },
    {
      "id": "decision-logs",
      "tasks": [
        "2.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no red-stage agent. NO-TESTER-WAIVER: same. Seam: the app log gains one info line per connection decision — connect, candidate start, candidate failed, auto-reconnect scheduled/exhausted, quit requested — with a test-only capturing logger added to crates/ui/src/logging.rs so ordering is assertable. Test files the coder may change: crates/ui/src/logging.rs (capture helper + `mod tests`), crates/ui/src/connection.rs (`mod tests`), crates/ui/src/app.rs (`mod tests`).",
      "contract": {
        "states": [
          "connect_recorded",
          "connect_not_recorded",
          "candidate_started",
          "candidate_failed",
          "failover_ended",
          "reconnect_scheduled",
          "reconnect_exhausted",
          "quit_recorded",
          "node_named_without_credentials"
        ],
        "transitions": [
          {
            "input": "start_connection reached with a planned candidate list",
            "state": "connect_recorded",
            "effect": "set",
            "evidence": "anchor '    fn start_connection(' — `connect origin=… strategy=… candidates=n profile_override=…`, the single choke point both Connect and ConnectToNode reach"
          },
          {
            "input": "start_connection refused by a guard (no binary, geodata missing, IPv6 gate)",
            "state": "connect_not_recorded",
            "effect": "no-op",
            "evidence": "anchor '                self.show_toast(\"No backend binary configured — check Preferences\");' — the record is written only after the guards pass"
          },
          {
            "input": "a candidate's manager is about to start",
            "state": "candidate_started",
            "effect": "set",
            "evidence": "anchor '            let candidate_address = candidate.node.address().to_string();' — `candidate start i/n label=…`"
          },
          {
            "input": "config generation failed for a candidate",
            "state": "candidate_failed",
            "effect": "set",
            "evidence": "anchor '                            &format!(\"config generation failed: {e}\"),' — same text as the CandidateFailure entry"
          },
          {
            "input": "start_with_connection returned a non-host-level Err",
            "state": "candidate_failed",
            "effect": "set",
            "evidence": "anchor '    impl CandidateFailure { fn new(label: &str, reason: &str, address: &str, port: u16) -> Self {' — reason = strip_ansi(reason)"
          },
          {
            "input": "the manager gave up after its crash budget",
            "state": "candidate_failed",
            "effect": "set",
            "evidence": "anchor '                            ProcessState::Error(msg) => {' inside the supervision loop — logged before the next candidate begins"
          },
          {
            "input": "host-level failure",
            "state": "failover_ended",
            "effect": "forced",
            "evidence": "anchor '                    if e.is_host_level() {' — the loop returns, so no later candidate line follows"
          },
          {
            "input": "schedule_auto_reconnect returns true",
            "state": "reconnect_scheduled",
            "effect": "set",
            "evidence": "anchor '        self.auto_reconnect_attempts += 1;' — `auto-reconnect scheduled attempt=i/3 delay=5s`"
          },
          {
            "input": "auto_reconnect_allowed refuses (pending_exit or attempts == MAX_AUTO_RECONNECTS)",
            "state": "reconnect_exhausted",
            "effect": "set",
            "evidence": "anchor '        if !auto_reconnect_allowed(self.pending_exit, self.auto_reconnect_attempts) {'"
          },
          {
            "input": "App::quit computes a plan",
            "state": "quit_recorded",
            "effect": "set",
            "evidence": "anchor '        let plan = quit_plan(' — `quit requested plan=…`"
          },
          {
            "input": "any decision record",
            "state": "node_named_without_credentials",
            "effect": "forced",
            "evidence": "requirement sentence: 'Records SHALL identify nodes by name and SHALL NOT include credentials'"
          }
        ],
        "forbidden": [
          "a candidate-failed line whose reason differs from the text in `CandidateFailure`'s summary (both come from the one already-stripped string)",
          "changing `terminal_failure(&failures)` or the `last_candidate_failure_reports_one_error` expected text",
          "logging the node's password, uuid, or subscription URL",
          "a connect line written for an attempt that never started",
          "installing the capturing logger outside `#[cfg(test)]`, or installing it more than once per test binary"
        ],
        "seeding": [
          "connect/auto-reconnect/quit lines: the app.rs formatter functions are free functions — unit-test them directly; app.rs tests never build an App",
          "reconnect_exhausted: call `auto_reconnect_allowed(false, MAX_AUTO_RECONNECTS)` directly, as `auto_reconnect_exhausted_after_three_attempts` does",
          "The capture is one process-wide logger per test binary: `candidate(\"203.0.113.1\"/.2/.3)` appears in at least eight other connection.rs tests, all multi_thread and concurrent under --test-threads=4. A test asserting on capture ORDER must use addresses unique to itself (198.51.100.11/.12/.13) and filter on them."
        ],
        "budgets": [
          "MAX_AUTO_RECONNECTS = 3 and AUTO_RECONNECT_DELAY = 5 s are the numbers the reconnect line prints",
          "one connect line per start_connection, one start line per candidate, at most one failed line per candidate",
          "`TestLogCapture::wait_for(needle, count)` polls at most 5 s at 50 ms",
          "log level: every decision line is `info`, i.e. visible at the default LevelFilter::Info"
        ]
      },
      "codeTasks": [
        "TEST FIRST (logging.rs): add `#[cfg(test)] pub(crate) struct TestLogCapture` over a `Mutex<Vec<String>>` implementing `Log`, `pub(crate) fn install_test_capture() -> &'static TestLogCapture` installing it once via `std::sync::Once` + `log::set_boxed_logger`, with `lines_containing(&self, needle: &str) -> Vec<String>` and `wait_for(&self, needle: &str, count: usize) -> Vec<String>` (5 s cap, 50 ms poll); unit-test the capture itself with two records",
        "TEST FIRST (connection.rs): add `failover_is_traceable` — three candidates, first fails, use addresses used by no other test in the binary — 198.51.100.11/.12/.13 — and filter the capture on those; assert it holds `candidate start 1/3` with 198.51.100.11, then `candidate failed 1/3` carrying its reason, then `candidate start 2/3`, in that order",
        "TEST FIRST (app.rs): add table tests for `connect_line`, `auto_reconnect_line`, `auto_reconnect_exhausted_line` and `quit_line` asserting the exact key=value text, including `candidates=3` and `attempt=1/3 delay=5s`",
        "Add the pure formatters in app.rs beside `quit_plan`: `fn connect_line(origin: ConnectOrigin, strategy: AutoResolveStrategy, candidates: usize, profile_override: bool) -> String`, `fn auto_reconnect_line(attempt: u32, max: u32) -> String`, `fn quit_line(plan: &QuitPlan) -> String`, and an `origin_field(ConnectOrigin) -> &'static str` mapping User→user (node when a direct target is set), AutoReconnect→auto-reconnect, Restart→restart",
        "Log the connect record in `start_connection` after the guards pass, using the `origin` parameter added in the status-text seam and `uses_imported_profile` over the candidate list for `profile_override`",
        "Log `auto-reconnect scheduled` / `auto-reconnect exhausted` in `schedule_auto_reconnect`, and `quit requested plan=…` in `App::quit` after `quit_plan` returns",
        "In connection.rs log `candidate start i/n label=…` at anchor '            let candidate_address = candidate.node.address().to_string();' and `candidate failed i/n label=… reason=…` at each `failures.push(CandidateFailure::new(` site, reusing the stripped reason"
      ]
    },
    {
      "id": "unclean-exit-paths",
      "tasks": [
        "3.1",
        "3.2",
        "3.3"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no red-stage agent. NO-TESTER-WAIVER: same. Seam: SIGTERM/SIGINT/SIGHUP run the normal quit path through the GLib loop, a chained panic hook records the panic, and a PID file left by a previous run is recorded in backend.log before the reap. Test files the coder may change: crates/ui/src/logging.rs (`mod tests`), crates/ui/src/app.rs (`mod tests`). The signal path itself is verified manually (task 5.2).",
      "contract": {
        "states": [
          "pending_exit=false",
          "pending_exit=true",
          "process_exited",
          "source_ids_stored",
          "panic_hook_installed",
          "panic_recorded",
          "previous_hook_ran",
          "stale_pid_present",
          "stale_pid_absent",
          "reap_skipped",
          "record_before_session"
        ],
        "transitions": [
          {
            "input": "SIGTERM|SIGINT|SIGHUP while pending_exit=false",
            "state": "pending_exit=true",
            "effect": "set",
            "evidence": "anchor '            AppMsg::TrayQuit => {' — the handler emits the same message the tray quit uses, and logs the signal"
          },
          {
            "input": "a second signal while pending_exit=true",
            "state": "process_exited",
            "effect": "forced",
            "evidence": "proposal sentence: 'a second signal while `pending_exit` is set exits immediately so a wedged stop cannot make the app unkillable'"
          },
          {
            "input": "init_logging returns during try_run",
            "state": "panic_hook_installed",
            "effect": "set",
            "evidence": "anchor '    crate::logging::init_logging(&paths);' — install_panic_hook() is called immediately after"
          },
          {
            "input": "startup with paths.pid_file_path() present",
            "state": "stale_pid_present",
            "effect": "set",
            "evidence": "anchor '                    if !skip_orphans && let Err(err) = cleanup_orphaned_backend(&orphan_paths) {' — the record is appended before the reap"
          },
          {
            "input": "startup without a PID file",
            "state": "stale_pid_absent",
            "effect": "no-op",
            "evidence": "design sentence: 'The PID file is removed on every clean stop and crash exit'"
          },
          {
            "input": "settings failed to load (skip_orphans = true)",
            "state": "reap_skipped",
            "effect": "forced",
            "evidence": "anchor '            let skip_orphans = settings_load_error.is_some();' — no reap and no record"
          }
        ],
        "forbidden": [
          "a tokio signal task instead of the GLib handler — the quit path owns GTK state and must run on the main loop",
          "a signal path that destroys the window without stopping the backend (it must reuse App::quit, so the exit record is written)",
          "a panic hook that replaces the previous hook instead of chaining it",
          "writing the unclean-previous-run record after `check_and_kill_orphaned` has removed the PID file, or from outside the lifecycle-lock block",
          "holding the short-lived RotatingFileWriter beyond the record"
        ],
        "seeding": [
          "panic record: call the pure `fn panic_record(info: &std::panic::PanicHookInfo) -> String` directly from a test that triggers it through `std::panic::catch_unwind`, and drive `AppLogger` with the existing `log_record` harness — do not install the global hook in a unit test",
          "the signal path has no unit seeding: it is manual verification (`kill -TERM` while connected, task 5.2)",
          "Seed the stale PID file with a LIVE but non-matching pid: `PidFile::new(paths.pid_file_path()).write(std::process::id(), Path::new(\"/bin/sh\"), &temp_config)`. `PidFile::write` builds the record through `process_start_time(pid)?`, so a dead pid returns Err(NotFound) and the test fails before asserting; serialising a PidOwnershipRecord directly is unavailable because crates/ui has no serde_json dependency. process_matches_record then fails, check_and_kill_orphaned removes the file and returns false, and the record reads `killed=false` with the pid file present before the reap."
        ],
        "budgets": [
          "3 signals handled: SIGTERM, SIGINT, SIGHUP",
          "exactly 1 unclean-previous-run record per app start",
          "exactly 1 panic record per panic",
          "backend.log writer opened at DEFAULT_MAX_BYTES, the same cap the connection task uses"
        ]
      },
      "codeTasks": [
        "TEST FIRST (logging.rs): add `panic_record_names_message_and_location` over the pure `panic_record`, and `install_panic_hook_chains_previous` asserting a sentinel set by a previously installed hook still fires",
        "Add `fn panic_record(info: &std::panic::PanicHookInfo) -> String` and `pub fn install_panic_hook()` (chaining `std::panic::take_hook`) to crates/ui/src/logging.rs; call `install_panic_hook()` right after `crate::logging::init_logging(&paths);`",
        "Install `glib::unix_signal_add_local` handlers for SIGTERM (15), SIGINT (2) and SIGHUP (1) in `App::init`, each logging the signal and emitting `AppMsg::TrayQuit`, returning `glib::ControlFlow::Continue`; store the SourceIds on the model; the second-signal force exit lives in `App::quit` behind the existing `pending_exit` flag",
        "TEST FIRST: add `force_exit_on_second_signal` table test — false on the first signal (pending_exit false), true on the second (pending_exit true)",
        "Add the pure `fn force_exit_on_second_signal(pending_exit: bool) -> bool` beside `quit_plan` and take the early exit path in App::quit when it returns true",
        "TEST FIRST (app.rs): add `stale_pid_file_records_unclean_previous_run` — temp profile, stale PID ownership record, call `record_unclean_previous_run(&paths)`, assert backend.log contains `unclean previous run left backend pid=` and the pid; add `no_pid_file_records_nothing`",
        "Extract `fn record_unclean_previous_run(paths: &AppPaths) -> bool` in app.rs: read the PID via `PidFile::read()` before the reap, run `cleanup_orphaned_backend`, and append `unclean previous run left backend pid=… killed=…` through a short-lived `RotatingFileWriter::open(paths.logs_dir().join(\"backend.log\"), DEFAULT_MAX_BYTES)`; call it in the startup block in place of the bare `cleanup_orphaned_backend` call"
      ]
    },
    {
      "id": "status-text",
      "tasks": [
        "4.1"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no red-stage agent. NO-TESTER-WAIVER: same. Seam: one pure status function over a `StatusView` input struct held by `App`, replacing the `Starting` arm of `update_status_labels`, so a crash respawn reads 'Restarting after crash' and an auto-reconnect reads 'Reconnecting (n/3)'. Test files the coder may change: crates/ui/src/app.rs (`mod tests`).",
      "contract": {
        "states": [
          "origin=User",
          "origin=AutoReconnect",
          "origin=Restart",
          "prev=Running",
          "prev=Stopped",
          "text=Connected",
          "text=RestartingAfterCrash",
          "text=Reconnecting",
          "text=Connecting",
          "text=unchanged",
          "status_not_updated",
          "attempt_preserved"
        ],
        "transitions": [
          {
            "input": "state=Running with metadata",
            "state": "text=Connected",
            "effect": "no-op",
            "evidence": "anchor '                (\"Connected\".to_string(), details)' — details string unchanged"
          },
          {
            "input": "state=Starting with prev=Running",
            "state": "text=RestartingAfterCrash",
            "effect": "set",
            "evidence": "anchor '        let from = self.process_state.clone();' in apply_state — the previous state is already captured there"
          },
          {
            "input": "state=Starting, prev != Running, origin=AutoReconnect",
            "state": "text=Reconnecting",
            "effect": "set",
            "evidence": "anchor '        self.auto_reconnect_attempts += 1;' in schedule_auto_reconnect — n is the attempt, denominator MAX_AUTO_RECONNECTS"
          },
          {
            "input": "state=Starting, prev != Running, origin=User or origin=Restart",
            "state": "text=Connecting",
            "effect": "no-op",
            "evidence": "anchor '            (ProcessState::Starting, _) => (\"Connecting…\".to_string(), \"Resolving nodes\".into()),'"
          },
          {
            "input": "start_connection called (new generation)",
            "state": "prev=Stopped",
            "effect": "forced",
            "evidence": "anchor '        self.apply_state(&ProcessState::Starting);' in start_connection — a fresh connect is not a respawn; origin is set from its new parameter"
          },
          {
            "input": "state=Stopping, Stopped or Error",
            "state": "text=unchanged",
            "effect": "no-op",
            "evidence": "anchor '            (ProcessState::Stopping, _) => {'"
          },
          {
            "input": "a superseded generation reports Starting",
            "state": "status_not_updated",
            "effect": "no-op",
            "evidence": "anchor '                if !is_current_generation(generation, self.connection_generation) {'"
          },
          {
            "input": "connect with origin=AutoReconnect",
            "state": "attempt_preserved",
            "effect": "no-op",
            "evidence": "anchor 'fn cancels_auto_reconnect(origin: ConnectOrigin) -> bool {' — `origin != ConnectOrigin::AutoReconnect`, so the counter is not reset"
          }
        ],
        "forbidden": [
          "two pure status functions over the same widget pair — one `status_texts(&StatusView) -> (String, String)` only (this supersedes tasks.md's `status_primary` name, per the sprint's one-status-function decision)",
          "'Restarting after crash' shown for a fresh connect or for a state reported by a superseded generation",
          "'Reconnecting (n/3)' with n read from anything but the single attempt accessor",
          "reading the health/dns fields in a non-Running arm (they belong to the sibling change and `App` clears them outside Running)"
        ],
        "seeding": [
          "`status_texts` is a free function over `StatusView` — build the struct literally in a table test beside `toggle_disabled_while_stopping`; app.rs tests never construct an App or a GTK widget",
          "connection metadata for the Running arms: the existing `snapshot` / `session_target_node` fixtures",
          "prev_state is reached only through `apply_state`'s captured `from` in production; a test sets the field on `StatusView` directly"
        ],
        "budgets": [
          "MAX_AUTO_RECONNECTS = 3 is the denominator printed in 'Reconnecting (n/3)'",
          "n ranges over 1..=3 — attempt 0 with origin AutoReconnect must still render 'Connecting…'"
        ]
      },
      "codeTasks": [
        "TEST FIRST: add `status_texts_covers_every_starting_case` — a table over StatusView asserting Restarting after crash / Reconnecting (1/3) / Connecting… and that the Running, Stopping, Stopped and Error arms are byte-identical to today's strings",
        "Add `struct StatusView { state: ProcessState, prev_state: ProcessState, meta: Option<ConnectionMetadata>, origin: ConnectOrigin, attempt: u32 }` and `fn status_texts(view: &StatusView) -> (String, String)` in app.rs, holding today's `update_status_labels` match plus the two new Starting arms (the sibling change adds `health` and `dns_failing` fields and the Running arms that read them)",
        "Add fields `status_prev: ProcessState` and `connection_origin: ConnectOrigin` to `App`; set `status_prev` from the `from` already captured in `apply_state`, and force it to `ProcessState::Stopped` in `start_connection` before its `apply_state(&ProcessState::Starting)`",
        "Add an `origin: ConnectOrigin` parameter to `fn start_connection(` and thread it from both call sites (the AppMsg::Connect arm and the AppMsg::ConnectToNode arm), storing it in `connection_origin`",
        "Rewrite `update_status_labels` to build a `StatusView` from the model and call `status_texts`"
      ]
    },
    {
      "id": "verification",
      "tasks": [
        "5.1",
        "5.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no red-stage agent. NO-TESTER-WAIVER: same. Seam: the whole-change floor plus the live pass that no unit test can cover (real backend, real signals, real TUN device). Task 5.2 is MANUAL.",
      "contract": {
        "states": [
          "floor_green",
          "live_verified"
        ],
        "transitions": [
          {
            "input": "make fmt && make clippy && make test TEST_TIMEOUT=10m",
            "state": "floor_green",
            "effect": "set",
            "evidence": "Makefile anchors 'fmt:', 'clippy:' and 'test:' — TEST := timeout $(TEST_TIMEOUT) $(CARGO) test, TEST_ARGS := -- --test-threads=$(TEST_THREADS)"
          },
          {
            "input": "live: connect, apply-with-restart, node switch, disconnect, quit",
            "state": "live_verified",
            "effect": "set",
            "evidence": "spec scenario 'Stop reasons are distinguished' — the four exit records read apply-restart, node-switch, user-stop, app-quit"
          },
          {
            "input": "live: kill -TERM while connected",
            "state": "live_verified",
            "effect": "set",
            "evidence": "tasks.md 3.1: 'verified live' — app log records the signal, backend.log has reason=app-quit, TUN device gone"
          }
        ],
        "forbidden": [
          "a bare `cargo test` without a timeout and a thread cap (the repo's own test-limits rule)",
          "claiming 5.2 done from unit tests — it needs a real backend and a real TUN device"
        ],
        "seeding": [
          "floor: run from the repo root against a clean working tree",
          "live: a real xray or sing-box binary with TUN granted; not reachable in CI"
        ],
        "budgets": [
          "workspace tests: timeout 10m, --test-threads=4",
          "per-crate tests: timeout 5m, --test-threads=4"
        ]
      },
      "codeTasks": [
        "Run the full floor and paste the failing output verbatim if anything is red",
        "MANUAL: execute the task 5.2 live pass and record the observed exit reasons and app-log lines"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "### Requirement: Backend output survives the application The system SHALL append every line the backend writes to stdout or stderr, and every line the route helper writes — including during a TUN route-recovery pass run outside a connection — to a backend log file under the state directory. Each backend launch SHALL be preceded by a session record naming the backend, its version, the node, whether TUN is on, the TUN DNS hijack mode, whether port-53 capture is installed, whether strict routing is on, whether every node hostname was pinned to addresses, and whether the node's routing and DNS came from an imported profile or the app settings. Each backend exit SHALL be followed by an exit record stating the exit code or signal, whether the stop was requested, the reason for the exit (`user-stop`, `node-switch`, `apply-restart`, `app-quit`, `start-failed`, or `crash`; `start-failed` SHALL cover both a start the application aborted and a backend that exited on its own before it was ready, and in the latter case the record SHALL read `requested=false`), the number of crashes in the current window, the last output line, and the last warning, error, or fatal line among the recent output (or `none`). When the application starts and finds a backend PID file left by a previous run, it SHALL append a record stating that the previous run ended without stopping its backend and whether a still-running backend was killed. A route-recovery pass SHALL additionally record its outcome (success, exit status, or timeout).",
      "tests": [
        "crates/process/src/manager.rs::tests::exit_record_marks_requested_stop",
        "crates/process/src/manager.rs::tests::last_error_survives_access_noise"
      ]
    },
    {
      "shall": "- **THEN** `<state_dir>/logs/backend.log` SHALL still contain the backend's last output lines and an exit record marking the exit as unrequested with reason `crash`",
      "tests": [
        "crates/process/src/manager.rs::tests::exit_record_marks_unrequested_crash"
      ]
    },
    {
      "shall": "- **THEN** every line SHALL still be written to the backend log file",
      "tests": [
        "crates/process/src/manager.rs::tests::last_error_survives_access_noise"
      ]
    },
    {
      "shall": "- **THEN** every line the route helper printed and the pass's outcome SHALL appear in `<state_dir>/logs/backend.log`, and none SHALL be written only to the application's standard error",
      "tests": [
        "crates/ui/src/app.rs::tests::stale_pid_file_records_unclean_previous_run",
        "MANUAL task 3.1: startup recovery pass output lands in backend.log"
      ]
    },
    {
      "shall": "- **THEN** the four exit records SHALL carry reasons `user-stop`, `apply-restart`, `node-switch`, and `app-quit` respectively",
      "tests": [
        "crates/process/src/manager.rs::tests::exit_record_reason_per_stop_reason",
        "crates/ui/src/app.rs::tests::stop_reason_for_maps_every_input",
        "crates/ui/src/connection.rs::tests::stop_reason_reaches_the_exit_record"
      ]
    },
    {
      "shall": "- **THEN** the exit record SHALL carry reason `start-failed`",
      "tests": [
        "crates/process/src/manager.rs::tests::exit_record_marks_start_failed_on_tun_device_timeout"
      ]
    },
    {
      "shall": "- **THEN** the exit record SHALL carry `requested=false`, reason `start-failed`, and `crashes_in_window=0`",
      "tests": [
        "crates/process/src/manager.rs::tests::exit_before_ready_records_start_failed (created by THIS change; #1's startup_failure_writes_one_session_and_no_crash predates `reason=` and asserts the session count and crashes_in_window=0 only)"
      ]
    },
    {
      "shall": "- **THEN** the exit record SHALL carry that warning line as `last_error` and the final access line as `last_output`",
      "tests": [
        "crates/process/src/manager.rs::tests::last_error_survives_access_noise",
        "crates/process/src/manager.rs::tests::last_error_is_none_without_warnings",
        "crates/process/src/manager.rs::tests::last_error_matches_through_ansi"
      ]
    },
    {
      "shall": "- **THEN** the session record SHALL state `hijack=hijack`, `capture_dns=true`, `nodes_pinned=true`, and `profile=app`",
      "tests": [
        "crates/ui/src/connection.rs::tests::xray_tun_session_record_carries_dns_decisions",
        "crates/process/src/manager.rs::tests::session_record_appends_caller_fields",
        "crates/core/src/models/imported_profile.rs::tests::uses_imported_profile_matches_resolve_effective_config"
      ]
    },
    {
      "shall": "- **THEN** `backend.log` SHALL contain a record stating the previous run ended without stopping its backend, before any new session record",
      "tests": [
        "crates/ui/src/app.rs::tests::stale_pid_file_records_unclean_previous_run",
        "crates/ui/src/app.rs::tests::no_pid_file_records_nothing"
      ]
    },
    {
      "shall": "### Requirement: Connection decisions are logged The application log SHALL record, at `info` level or above: each connection attempt with its origin (`user`, `node`, `auto-reconnect`, or `restart`), the resolve strategy, the number of candidates, and whether any candidate uses an imported profile; each candidate start with its position and node name; each candidate failure with its position, node name, and failure reason before the next candidate starts; each auto-reconnect scheduled with its attempt number and limit, and when the limit is exhausted; each backend state transition; each crash respawn with its crash count; and each quit request. Records SHALL identify nodes by name and SHALL NOT include credentials.",
      "tests": [
        "crates/ui/src/connection.rs::tests::failover_is_traceable",
        "crates/ui/src/app.rs::tests::connect_line",
        "crates/ui/src/app.rs::tests::auto_reconnect_line",
        "crates/ui/src/app.rs::tests::quit_line",
        "crates/process/src/state.rs::tests::rejected_transition_changes_nothing"
      ]
    },
    {
      "shall": "- **THEN** the app log SHALL contain the connect record with `candidates=3`, a failure record for candidate 1 with its reason, and a start record for candidate 2, in that order",
      "tests": [
        "crates/ui/src/connection.rs::tests::failover_is_traceable"
      ]
    },
    {
      "shall": "- **THEN** the app log SHALL contain a record with the attempt number and the limit of 3, followed by a connect record with origin `auto-reconnect` when it fires",
      "tests": [
        "crates/ui/src/app.rs::tests::auto_reconnect_line",
        "crates/ui/src/app.rs::tests::auto_reconnect_exhausted_line"
      ]
    },
    {
      "shall": "- **THEN** the app log SHALL contain a crash-respawn record with the exit code or signal and the crash count, and state-transition records for `Running` → `Starting` → `Running`",
      "tests": [
        "crates/process/src/manager.rs::tests::crash_respawn_records_crash_count",
        "crates/process/src/state.rs::tests::rejected_transition_changes_nothing"
      ]
    },
    {
      "shall": "### Requirement: Status bar distinguishes restarts from connects While a connection is starting, the status bar SHALL distinguish why: an in-place respawn after the backend exited unexpectedly SHALL show \"Restarting after crash\", and an attempt started by auto-reconnect SHALL show \"Reconnecting (n/3)\" with the current attempt number. Other starts SHALL keep showing \"Connecting…\".",
      "tests": [
        "crates/ui/src/app.rs::tests::status_texts_covers_every_starting_case"
      ]
    },
    {
      "shall": "- **THEN** the status bar SHALL show \"Restarting after crash\" until the backend is running again or the connection ends",
      "tests": [
        "crates/ui/src/app.rs::tests::status_texts_covers_every_starting_case"
      ]
    },
    {
      "shall": "- **THEN** the status bar SHALL show \"Reconnecting (2/3)\"",
      "tests": [
        "crates/ui/src/app.rs::tests::status_texts_covers_every_starting_case"
      ]
    },
    {
      "shall": "- **THEN** the status bar SHALL show \"Connecting…\"",
      "tests": [
        "crates/ui/src/app.rs::tests::status_texts_covers_every_starting_case"
      ]
    },
    {
      "shall": "### Requirement: Termination signals and panics leave a record The application SHALL handle SIGTERM, SIGINT, and SIGHUP by running the same quit path as a user quit, so a running backend is stopped, its exit record is written with reason `app-quit`, and TUN state is released before the process exits. A panic in the application SHALL be written to the application log with its message and location before the default panic behavior continues.",
      "tests": [
        "MANUAL task 3.1: kill -TERM while connected — app log records the signal, backend.log carries reason=app-quit, TUN device gone"
      ]
    },
    {
      "shall": "- **THEN** the app log SHALL record the signal, the backend SHALL be stopped gracefully, and `backend.log` SHALL contain an exit record with reason `app-quit`",
      "tests": [
        "crates/ui/src/app.rs::tests::stop_reason_for_maps_every_input",
        "MANUAL task 3.1 live signal pass"
      ]
    },
    {
      "shall": "- **THEN** the app log file SHALL contain an `error` record with the panic message and source location",
      "tests": [
        "crates/ui/src/logging.rs::tests::panic_record_names_message_and_location",
        "crates/ui/src/logging.rs::tests::install_panic_hook_chains_previous"
      ]
    }
  ],
  "testHarness": [
    "write_script — crates/process/src/manager.rs — an executable /bin/sh `backend` script in a temp dir",
    "manager_for — crates/process/src/manager.rs — a ProcessManager over that stub with config.json and backend.pid in the temp dir",
    "backend_log — crates/process/src/manager.rs — Arc<RotatingFileWriter> over <dir>/backend.log at DEFAULT_MAX_BYTES",
    "VERSION_STUB / SINGBOX_VERSION_STUB — crates/process/src/manager.rs — script prefixes answering `version` as Xray 26.3.27 / sing-box 1.13.0",
    "stub_helper — crates/process/src/manager.rs — a fake netctl recording its subcommand into <dir>/calls, returns (helper, calls)",
    "xray_on_lo — crates/process/src/manager.rs — a TunRuntime on iface lo pointed at a stub helper",
    "crashing_backend — crates/process/src/manager.rs — a script body that exits 1 for the first N runs then sleeps",
    "read_lines / wait_for_lines — crates/process/src/manager.rs — reads a log file; polls up to 2 s for at least n lines",
    "drain_states — crates/process/src/manager.rs — Vec<ProcessState> drained from a broadcast receiver",
    "contains_line — crates/process/src/manager.rs — predicate over the manager's last 10 buffered log lines",
    "HostProbe / with_host_probe — crates/process/src/manager.rs — pins getcap and helper paths so TUN capability gates are deterministic",
    "Stub / stub() — crates/ui/src/connection.rs — TempDir + AppPaths::for_profile_in(AppProfile::Test, …) + an executable stub backend",
    "executable — crates/ui/src/connection.rs — a trivial exit-0 executable at dir/name",
    "candidate / xhttp_candidate / node — crates/ui/src/connection.rs — ConnectionCandidate over a Shadowsocks (or XHTTP VLESS) node at the given address",
    "request / connect / connect_with — crates/ui/src/connection.rs — a ConnectionRequest (generation 7, host_has_ipv6 true) and spawns it with a relm4 channel; connect_with injects a per-manager configure hook",
    "capless_probe — crates/ui/src/connection.rs — configure hook attaching HostProbe{getcap:/bin/true, helper:/bin/true}",
    "next_state / drain / assert_nothing_after_terminal — crates/ui/src/connection.rs — awaits AppMsg::ProcessStateConnection with a 20 s cap; drain also collects every ProcessLogLine",
    "singbox_settings / tun_settings / strict_route_settings / v2ray_settings — crates/ui/src/connection.rs — AppSettings variants per backend and TUN mode",
    "attempts — crates/ui/src/connection.rs — counts lines a stub appended to a marker file (one per candidate attempt)",
    "log_record — crates/ui/src/logging.rs — drives AppLogger directly with a synthetic Record — the closest thing to a capturing logger that exists",
    "snapshot / session_target_node / routing_rule / profile_sub — crates/ui/src/app.rs — fixtures for the pure-function tests; app.rs tests never construct an App or a GTK widget"
  ],
  "floor": "make fmt && make clippy && make test TEST_TIMEOUT=10m   (fmt = cargo fmt -- --check; clippy = cargo clippy --workspace --all-targets --all-features -- -D warnings; test = timeout 10m cargo test --workspace --all-targets -- --test-threads=4). Plus the MANUAL task 5.2 live pass: connect, apply-with-restart, node switch, disconnect, quit, and a kill -TERM while connected.",
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
