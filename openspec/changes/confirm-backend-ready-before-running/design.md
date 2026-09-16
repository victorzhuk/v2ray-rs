## Context

- `crates/process/src/manager.rs:262-265`: `start_with_connection` transitions to `Running` right after `launch()` returns, and `launch()` returns once the child is spawned (plus, for xray TUN, device wait and `xray-up`, `:373-388`). `respawn()` (`:348-353`) does the same.
- `manager.rs:338-340`: any exit while the state is `Running` goes to `handle_unexpected_exit` (`:619-675`): records a crash, sleeps `CRASH_RESTART_DELAY * crashes` (2 s, then 4 s), respawns; `MAX_CRASHES` 3 within `CRASH_WINDOW` 60 s gives up with `Error("3 crashes within 60s: …")`.
- `crates/ui/src/connection.rs:215-226` reports `Running` with metadata after `start_with_connection` returns `Ok`; `:279-289` fails over when the manager gives up in `Error`. A start `Err` (`:227-230`) fails over immediately.
- `crates/ui/src/app.rs:1209-1216` persists `last_success` for every `ProcessStateConnection` message that carries metadata, whatever the state; respawn `Starting`/`Running` relays (`connection.rs:236-248`) carry it too.
- `backend.log`, 2026-09-15: `09:37:43.266 session backend=sing-box … tun=on`, `09:37:43.370 exit requested=false code=1 … FATAL[0000] start service: post-start inbound/tun[tun-in] …`; next sessions at `:45.372` and `:49.450`; three sessions per candidate, three candidates.
- The failing phase is `post-start`: sing-box has already started its inbounds when it FATALs, so the mixed inbound may accept a connection for a few milliseconds.
- Inbounds: sing-box `mixed` on `socks_port` (`crates/core/src/config/singbox.rs:162-176`), xray/v2ray `socks` on `socks_port` (`crates/core/src/config/v2ray.rs:155-169`), all on `listen_address` (an IP literal).
- `crates/process/src/probe.rs:92-119` already polls a loopback port while watching the child for an early exit (`ProbeRunner::wait_ready`).

## Goals / Non-Goals

**Goals:**
- `Running` and last-success mean the backend is actually serving its inbound.
- A backend that dies at startup costs one launch per candidate, not three plus 6 s of delays.

**Non-Goals:**
- Proving end-to-end proxy reachability (the node may still be dead); that is `detect-dead-proxy-link`.
- Stopping failover for identical failures across candidates; `stop-failover-on-shared-failure` owns that rule and compares the startup-failure reason produced here.
- A settings-apply restart race. The reported sequence (`09:26:48.566 session`, `09:26:48.602 exit requested=true signal=15`) is preceded and followed by `Regenerated config` records, which `app.rs` writes only after a stop with no reconnect pending (`:1238-1240`) or a settings flush with no connection (`:1029-1031`); Apply & Restart sets `reconnect_pending`, skips regeneration and reconnects at once (`:1425-1428`, `:1241-1243`). The SIGTERM was a Disconnect during `Starting` (the button shows Disconnect then, `:1473-1478`), which "Stop during start" already specifies. No debounce is added.

## Decisions

- **Ready = inbound accepts a TCP connection AND the process is alive `STABILITY_WINDOW` (1 s) after spawn.** Port alone passes a sing-box that binds inbounds and then FATALs in post-start; a window alone passes a backend that stays alive without listening (e.g. blocked on a remote rule-set fetch). Both conditions are cheap and backend-agnostic. The observed FATALs land ~100 ms after spawn, so 1 s has a 10× margin. Alternatives rejected: parsing a backend "started" log line (differs per backend and version); probing through the proxy (depends on the node, not the backend).
- **Probe address.** `listen_address:socks_port` from the candidate's effective settings; an unspecified address (`0.0.0.0`, `::`) is probed on the matching loopback. Passed to the manager with a builder next to `with_tun`; a manager without it keeps today's behavior (spawn = ready), so `ProbeRunner`-style and unit users are unaffected.
- **`READY_TIMEOUT` 15 s, polled every 100 ms.** Covers xray loading large `geosite.dat` and a sing-box first-run rule-set fetch; the attempt stays cancellable from `Starting`. On timeout the backend is stopped and the start fails with `backend did not accept connections on <addr> within 15s`.
- **Exit before ready on the initial start is a startup failure.** `start_with_connection` returns a new `ProcessError::ExitedBeforeReady` carrying `process exited with code N: <last output line>` (same shape as a crash reason, so the shared-failure key still normalizes). No crash is recorded, no respawn, state goes `Starting → Error`, and the connection task fails over through its existing `Err` branch. Alternative rejected: keep respawning but faster — a deterministic startup FATAL repeats identically.
- **Respawn keeps crash semantics.** A session that was ready and crashed may hit a transient fault; its respawn waits for readiness and an exit or timeout before ready is recorded as a crash and retried, per "Failed respawn is retried". Routing state stays in place as today.
- **xray TUN ordering.** spawn → device wait → `xray-up` → readiness. The stability window counts from spawn, so the helper time is not added twice. A readiness failure stops the backend like a helper failure does.
- **Last-success only on `Running`.** `app.rs` persists `last_success` when the state is `Running` with metadata; `Starting`/`Stopping` relays with metadata update the displayed status only. Since `Running` now implies readiness, this is the whole spec change for `connection-auto-resolve`.

## Risks / Trade-offs

- [A backend with no socks/mixed inbound on `socks_port`] → every generator emits it unconditionally today; the readiness probe names the address in its error if a future config drops it.
- [Another process already holds `socks_port`] → the probe connects to that process; the backend then fails to bind and exits inside the stability window, which is detected. A backend that logs the bind error and keeps running is reported ready — no worse than today.
- [Slow hosts exceed 15 s] → failure names the timeout; the constant is a single place to raise. Cancel works throughout.
- [Existing stub-backend tests expect `Running` from `exec sleep 30`] → they bind a local listener and set `socks_port` to it; tests that relied on three crash respawns before failover change their expected reason.
- [Probe connection shows up in backend access logs] → one connection per start; acceptable.

## Implementation plan

Tier `heavy`, mode `existing-service-strict`, lenses `spec`, `quality`, `arch`. Planned at `1d6a0fb8`.

Rust has no red-stage test-writer agent, so every seam carries `NO-RED-WAIVER` / `NO-TESTER-WAIVER` and its tests are the first `codeTasks` of the chunk that owns them.

This change lands first in sprint `trustworthy-connection-state` (after the free-standing xray one). `log-connection-decisions` and `detect-dead-proxy-link` rebase onto its edits in `crates/process/src/manager.rs` and `crates/ui/src/connection.rs`.


### `r1` — tasks 1.1 — seam `core-local-endpoint`

- order: parallel, shard `core`; coder `rust-coder`; packages `v2ray-rs-core`
- sites:
  - `crates/core/src/models/settings.rs` · `AppSettings::local_endpoint (new)` · anchor `pub fn validate_listen_address(addr: &str) -> Result<(), ValidationError> {` — Add `pub fn local_endpoint(&self, port: u16) -> SocketAddr` to the same impl block; unspecified v4/v6 maps to the loopback of that family, parse failure falls back to Ipv4Addr::LOCALHOST.
  - `crates/core/src/models/settings.rs` · `tests::local_endpoint_maps_unspecified_to_loopback (new)` · anchor `fn test_validate_listen_address()` — Add the sibling unit test asserting the five rows: 127.0.0.1, 0.0.0.0, ::, 192.168.1.10, and an unparsable literal.
- work, in order:
  - 1.1-t1: in crates/core/src/models/settings.rs `mod tests`, next to `fn test_validate_listen_address()`, add `fn local_endpoint_maps_unspecified_to_loopback()` asserting the five rows of the transition table as SocketAddr values (127.0.0.1:1080, 127.0.0.1:1080, [::1]:1080, 192.168.1.10:1080, 127.0.0.1:1080)
  - 1.1-t2: add `pub fn local_endpoint(&self, port: u16) -> SocketAddr` to the `impl AppSettings` block that holds validate_listen_address; parse self.listen_address with IpAddr::from_str (already imported via `use std::str::FromStr;` and `use std::net::IpAddr;`), map IpAddr::is_unspecified to Ipv4Addr::LOCALHOST / Ipv6Addr::LOCALHOST of the same family, and fall back to Ipv4Addr::LOCALHOST on a parse error; extend the `use std::net::` line to bring in SocketAddr, Ipv4Addr, Ipv6Addr
  - 1.1-t3: `make test-core` green
- verify: `make test-core && cargo clippy -p v2ray-rs-core --all-targets -- -D warnings`

### `r2` — tasks 1.2 — seam `process-readiness`

- order: after `r1` (compile dependency, no shared package); coder `rust-coder`; packages `v2ray-rs-process`
- sites:
  - `crates/process/src/manager.rs` · `ProcessError` · anchor `#[error("config rejected by backend: {0}")]` — add variants: ExitedBeforeReady(String) and a readiness-timeout variant naming address + duration; both non-host-level
  - `crates/process/src/manager.rs` · `ProcessManager fields` · anchor `restart_delay: Duration,` — add ready_probe: Option<SocketAddr> and ready_timeout: Duration (test-settable, same shape as restart_delay)
  - `crates/process/src/manager.rs` · `ProcessManager::new` · anchor `restart_delay: CRASH_RESTART_DELAY,` — initialise ready_probe: None and ready_timeout: READY_TIMEOUT (15 s)
  - `crates/process/src/manager.rs` · `ProcessManager::with_ready_probe` · anchor `pub fn with_tun(mut self, tun: Option<TunRuntime>) -> Self {` — add builder with_ready_probe(SocketAddr) next to with_tun
  - `crates/process/src/manager.rs` · `ProcessManager::wait_ready (new)` · anchor `// A launch failure leaves routing state in place: during a respawn the` — add async wait_ready() after launch(): poll TCP connect every 100 ms, require child alive STABILITY_WINDOW (1 s) after spawn; on child exit build the reason like handle_unexpected_exit and return ExitedBeforeReady; on timeout return the timeout error; no-op Ok when ready_probe is None
  - `crates/process/src/manager.rs` · `exit reason formatting` · anchor `let mut msg = match exit_code {` — reuse/extract this `process exited with code N: <last line>` shape so ExitedBeforeReady carries the identical text (shared-failure key normalisation depends on it)
  - `crates/process/src/manager.rs` · `mod tests (wait_ready)` · anchor `fn manager_for(dir: &tempfile::TempDir, script_body: &str) -> ProcessManager {` — add tests using manager_for + a test-owned TcpListener: `exec sleep 30` + listener -> ready; `echo FATAL; exit 1` -> ExitedBeforeReady containing FATAL; `exec sleep 30` with no listener and 300 ms ready_timeout -> timeout error and child reaped
- work, in order:
  - 1.2-t1: test `ready_probe_accepts_live_backend` — listener alive, `exec sleep 30` stub, with_ready_probe(listener addr); start() -> Ok, mgr.state() == ProcessState::Running; assert the start took at least STABILITY_WINDOW
  - 1.2-t2: test `exit_before_ready_carries_last_output` — stub `echo FATAL >&2` then `exit 1`, live listener, with_ready_probe; start() -> Err(ProcessError::ExitedBeforeReady(msg)) with msg containing "code 1" and "FATAL"
  - 1.2-t3: test `ready_timeout_stops_the_backend` — `exec sleep 30` stub, probe address from a bound-then-dropped listener, mgr.ready_timeout = 300 ms; start() -> Err(ProcessError::ReadyTimeout { .. }) whose Display names the address and `300ms`; mgr.child.is_none() afterwards (the child was reaped by graceful_stop)
  - 1.2-c1: add `const READY_TIMEOUT: Duration = Duration::from_secs(15);`, `const STABILITY_WINDOW: Duration = Duration::from_secs(1);` and `const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);` beside the anchor 'const CRASH_RESTART_DELAY: Duration = Duration::from_secs(2);'
  - 1.2-c2: add to the ProcessError enum, after the anchor '    #[error("config rejected by backend: {0}")]' variant: `#[error("{0}")] ExitedBeforeReady(String)` and `#[error("backend did not accept connections on {addr} within {timeout:?}")] ReadyTimeout { addr: SocketAddr, timeout: Duration }`; import std::net::SocketAddr
  - 1.2-c3: extend the `is_host_level_classifies_every_variant` case table with both new variants mapped to false (the test enumerates every variant and is the project's standing guard)
  - 1.2-c4: add fields `ready_probe: Option<SocketAddr>` and `ready_timeout: Duration` beside the anchor '    restart_delay: Duration,'; initialise them to None and READY_TIMEOUT beside the anchor '            restart_delay: CRASH_RESTART_DELAY,'
  - 1.2-c5: add `pub fn with_ready_probe(mut self, addr: SocketAddr) -> Self` next to the anchor '    pub fn with_tun(mut self, tun: Option<TunRuntime>) -> Self {', following the consuming-builder pattern
  - 1.2-c6: extract the exit-reason text at the anchor '        let mut msg = match exit_code {' into a private helper (status + last_output_line -> String) and call it from both handle_unexpected_exit and wait_ready, so the ExitedBeforeReady text is identical to a crash reason and failure_key normalisation keeps working
  - 1.2-c7: add `async fn wait_ready(&mut self) -> Result<(), ProcessError>` after the anchor '    // A launch failure leaves routing state in place: during a respawn the' — Ok(()) at once when ready_probe is None; otherwise record `probe_start: Instant` at wait_ready entry, loop every READY_POLL_INTERVAL: child.try_wait() Some(status) -> cleanup_after_exit().await, write_exit_record(false, Some(&status)), Err(ExitedBeforeReady(reason)); tokio::time::timeout(READY_POLL_INTERVAL, TcpStream::connect(addr)) Ok and probe_start.elapsed() >= STABILITY_WINDOW -> Ok(()) — an unbounded connect to a non-loopback address behind a drop rule outlives ready_timeout and the 15 s budget stops being enforced; deadline passed -> graceful_stop().await, Err(ReadyTimeout { addr, timeout: self.ready_timeout })
- verify: `make test-process && cargo clippy -p v2ray-rs-process --all-targets -- -D warnings`

### `r3` — tasks 1.3 — seam `process-readiness`

- order: after `r2` (shared `crates/process/src`); coder `rust-coder`; packages `v2ray-rs-process`
- sites:
  - `crates/process/src/manager.rs` · `ProcessManager::start_with_connection` · anchor `match self.launch().await {` — on Ok(()) call wait_ready() before transitioning to Running; on readiness failure graceful_stop the child, write the exit record without record_crash, transition Starting -> Error(e), return Err(e)
  - `crates/process/src/manager.rs` · `mod tests (startup failure)` · anchor `async fn crash_error_includes_last_stderr_line() {` — add test: FATAL-then-exit stub with a ready probe -> Err(ExitedBeforeReady), one ` session ` record in backend.log, crashes_in_window=0 in the exit record, no respawn after 3 s
- work, in order:
  - 1.3-t1: test `startup_failure_writes_one_session_and_no_crash` — FATAL-then-exit stub with .with_log_file(Some(backend_log(dir.path()))) and a ready probe; start() -> Err(ExitedBeforeReady); backend.log holds exactly one " session " record; the exit record contains "exit requested=false", "code=1", "crashes_in_window=0" and "last_output=FATAL"; after sleeping 3 s the log still holds exactly one session record (no respawn)
  - 1.3-c1: in start_with_connection at the anchor '        match self.launch().await {', change the Ok(()) arm to `self.wait_ready().await` first and transition to Running only on its Ok; on Err reuse the existing Err arm shape — transition to ProcessState::Error(e.to_string()) and return Err(e), with no record_crash and no graceful_stop of an already-reaped child
- verify: `make test-process && cargo clippy -p v2ray-rs-process --all-targets -- -D warnings`

### `r4` — tasks 1.4, 1.5 — seam `process-readiness`

- order: after `r3` (shared `crates/process/src`); coder `rust-coder`; packages `v2ray-rs-process`
- sites:
  - `crates/process/src/manager.rs` · `ProcessManager::respawn` · anchor `async fn respawn(&mut self) -> Result<(), ProcessError> {` — call wait_ready() after launch() and before the Running transition; its Err propagates so handle_unexpected_exit's loop records a crash and retries
  - `crates/process/src/manager.rs` · `mod tests (respawn readiness)` · anchor `async fn respawn_budget_exhaustion_errors() {` — add sibling test: stub serves once, crashes, then exits immediately on every respawn -> Error("3 crashes within 60s: …") after two respawns
  - `crates/process/src/manager.rs` · `mod tests (existing)` · anchor `async fn failed_respawn_is_retried() {` — must stay green unchanged: no ready probe set -> wait_ready is a no-op and spawn still means Running
- work, in order:
  - 1.4-t1: test `respawn_readiness_failure_exhausts_crash_budget` — stub that runs `exec sleep 30` on run 1 and exits 1 on every later run, live listener, ready probe, restart_delay 50 ms; loop wait_and_handle_exit() while Running; end state ProcessState::Error(msg) with msg containing "3 crashes within 60s"
  - 1.5-t1: assert the untouched path — run the existing suite; `failed_respawn_is_retried`, `respawn_skips_preflight`, `respawn_budget_exhaustion_errors`, `config_check_success_starts_backend`, `exit_record_marks_requested_stop`, `exit_record_marks_unrequested_crash` stay byte-identical and green
  - 1.4-c1: in respawn (anchor '    async fn respawn(&mut self) -> Result<(), ProcessError> {') insert `self.wait_ready().await?;` between `self.launch().await?;` and the Running transition
- verify: `make test-process && cargo clippy -p v2ray-rs-process --all-targets -- -D warnings`

### `r5` — tasks 2.1, 2.2 — seam `ui-connection-ready-probe`

- order: after `r4` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/connection.rs` · `candidate manager builder chain` · anchor `.with_log_file(backend_log.clone()),` — Append `.with_ready_probe(effective_settings.local_endpoint(effective_settings.socks_port))` to the chain — the only non-test production edit of task 2.1.
  - `crates/ui/src/connection.rs` · `tests::singbox_settings / stub harness` · anchor `fn singbox_settings() -> AppSettings {` — stub-backend tests bind an ephemeral listener and set settings.socks_port to its port so the readiness probe succeeds; add the helper next to this builder
  - `crates/ui/src/connection.rs` · `tests::last_candidate_failure_reports_one_error` · anchor `assert!(msg.contains("203.0.113.1: 3 crashes"), "{msg}");` — expect the startup-failure reason (process exited with code …) instead of `3 crashes`
  - `crates/ui/src/connection.rs` · `tests (new two-candidate readiness test)` · anchor `async fn live_connect_writes_backend_diagnostics() {` — add test beside it: candidate 1 exits with FATAL right after start, candidate 2 serves -> exactly one ` session ` record for candidate 1, Running only reported for candidate 2
  - `crates/ui/src/connection.rs` · `supervision loop exit paths (halt set)` · anchor `halt(state_forwarder).await; ⏎             halt(log_forwarder).await; ⏎             parked = Some(mgr);` — The fall-through already halts both; the Stop arm and the `_ =>` arm inside the loop halt only state_forwarder. Replace all three with one helper that halts every spawned task, so detect-dead-proxy-link can register its health monitor with it.
- work, in order:
  - 2.1-t1: add a harness helper next to the anchor '    fn singbox_settings() -> AppSettings {' that binds a std::net::TcpListener on 127.0.0.1:0, returns it together with AppSettings whose socks_port is the bound port, and document that the listener must outlive the connect
  - 2.1-t2: rewrite the settings of every test that expects Running to use that helper: failover_reports_no_stopped_and_stop_reports_one, log_lines_carry_connection_generation, live_connect_writes_backend_diagnostics
  - 2.1-t3: at the anchor '        assert!(msg.contains("203.0.113.1: 3 crashes"), "{msg}");' in last_candidate_failure_reports_one_error, expect the startup-failure reason instead — the candidate now fails once with `process exited with code 1` and never reaches the crash budget; keep the second assertion on "203.0.113.3: config rejected" as is
  - 2.2-t1: new test `two_candidates_first_exits_before_ready` beside the anchor '    async fn live_connect_writes_backend_diagnostics() {' — one shared listener; candidate 203.0.113.1's run branch prints FATAL and exits 1, candidate 203.0.113.2 runs `exec sleep 30`; assert Running is reported only with metadata naming 203.0.113.2, that no Running carried 203.0.113.1, and that backend.log holds exactly two " session " records (one per candidate attempt) with only one preceding the first candidate's exit record
  - 2.1-c1: append `.with_ready_probe(effective_settings.local_endpoint(effective_settings.socks_port))` to the builder chain at the anchor '                .with_log_file(backend_log.clone()),'
  - 2.1-c2: no import change is needed for the address helper (AppSettings is already imported); the process-crate `use v2ray_rs_process::{ProcessError, ProcessEvent, ProcessManager, ProcessState, TunRuntime};` line stays as is
  - 2.1-c3: replace the ad-hoc halts in the supervision loop with one helper that halts both state_forwarder and log_forwarder, and call it on every exit path that reports a terminal state — the Stop arm (anchor '                        halt(state_forwarder).await;'), the `_ =>` arm, and the existing fall-through after the loop; detect-dead-proxy-link registers its health task with the same helper
  - 2.1-c4: `make test-ui` green
  - 2.1-t4: add `stop_halts_every_forwarder` — connect a stub, `handle.stop()`, wait for Stopped, then assert no further backend log lines arrive on the buffer after the terminal report (the log forwarder is halted, not just the state forwarder)
- verify: `make test-ui && cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings`

### `r6` — tasks 2.3 — seam `ui-app-last-success`

- order: after `r5` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/app.rs` · `records_last_success (new pure fn)` · anchor `fn last_success_settings(current: &AppSettings, meta: &ConnectionMetadata) -> AppSettings {` — add pure records_last_success(&ProcessState, Option<&ConnectionMetadata>) -> bool next to it
  - `crates/ui/src/app.rs` · `AppMsg::ProcessStateConnection arm` · anchor `let settings = last_success_settings(&self.settings, meta);` — gate the persist on records_last_success(&state, self.connection_status.as_ref()) — `connection` is moved by the assignment above, so the `connection.as_ref()` form does not compile; Starting/Stopping relays still update connection_status only
  - `crates/ui/src/app.rs` · `tests` · anchor `fn direct_session_success_records_last_success() {` — add unit test for records_last_success: Starting+meta false, Running+meta true, Running+none false
- work, in order:
  - 2.3-t1: test `records_last_success_only_on_running` beside the anchor '    fn direct_session_success_records_last_success() {' asserting the three rows, plus Stopping+meta false
  - 2.3-c1: add `fn records_last_success(state: &ProcessState, connection: Option<&ConnectionMetadata>) -> bool` next to the anchor 'fn last_success_settings(current: &AppSettings, meta: &ConnectionMetadata) -> AppSettings {' — true only for `(ProcessState::Running, Some(_))`
  - 2.3-c2: gate the persist at the anchor '                        let settings = last_success_settings(&self.settings, meta);' on records_last_success(&state, self.connection_status.as_ref()) — computing it from `connection` after `self.connection_status = connection;` borrows a moved value and does not compile; leave the `self.connection_status = connection;` assignment above it untouched
  - 2.3-c3: `make test-ui` green
- verify: `make test-ui && cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings`

### `r7` — tasks 3.1, 3.2 — seam `verification`

- order: after `r6` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-core`, `v2ray-rs-process`, `v2ray-rs-ui`
- work, in order:
  - 3.1-t1: `make test TEST_TIMEOUT=10m` green (equivalently `timeout 10m cargo test --workspace --all-targets -- --test-threads=4`)
  - 3.1-t2: `make lint` green (cargo fmt --check plus cargo clippy --workspace --all-targets --all-features -- -D warnings)
  - 3.2-m1: MANUAL — sing-box TUN on an IPv6-less host: status never reaches Connected, backend.log has one ' session ' per candidate with no 2 s/4 s respawn pairs, settings.toml last_success unchanged
  - 3.2-m2: MANUAL — xray TUN on a working node: Connected only after the device wait and xray-up succeed, and settings.toml last_success updated
- verify: `make test TEST_TIMEOUT=10m && make lint`

### Contracts


**`core-local-endpoint`** — tasks 1.1
- states: `loopback-v4`, `unspecified-v4`, `unspecified-v6`, `explicit-v4`, `unparsable`
- transitions:
  - `listen_address "127.0.0.1", port 1080` → `loopback-v4` → **no-op** — anchor 'pub fn validate_listen_address(addr: &str) -> Result<(), ValidationError> {' — the literal is already a probeable address; local_endpoint returns 127.0.0.1:1080
  - `listen_address "0.0.0.0", port 1080` → `unspecified-v4` → **forced** — requirement sentence: 'An unspecified listen address SHALL be probed on the loopback address of the same family.' — forced to 127.0.0.1:1080
  - `listen_address "::", port 1080` → `unspecified-v6` → **forced** — requirement sentence: 'An unspecified listen address SHALL be probed on the loopback address of the same family.' — forced to [::1]:1080
  - `listen_address "192.168.1.10", port 1080` → `explicit-v4` → **no-op** — anchor '        let valid = ["127.0.0.1", "0.0.0.0", "::", "::1", "192.168.1.10"];' — an explicit bind address is probed as written
  - `listen_address that IpAddr::from_str rejects ("", "localhost")` → `unparsable` → **forced** — anchor '        IpAddr::from_str(addr)' — validate_listen_address rejects these at edit time, but a loaded settings.toml can still carry one; local_endpoint forces 127.0.0.1:<port> instead of panicking or unwrapping
- forbidden:
  - local_endpoint returning an unspecified address (0.0.0.0 or ::) — nothing can TCP-connect to it
  - local_endpoint panicking or returning Result — every caller sits on a start path and has no recovery to offer
  - a second address formatter: crates/ui/src/connection.rs keeps `fn listen_endpoint(addr: &str, port: u16) -> String` for the human-facing v2ray warning text (bracketed display form), and it is NOT replaced by local_endpoint in this change
- seeding:
  - every state is reached by constructing AppSettings::default() and assigning settings.listen_address plus the port argument; no file I/O, no persistence
- budgets:
  - pure function, no await, no syscall — 0 ms of wall clock
  - 5 assertion cases: 127.0.0.1, 0.0.0.0, ::, 192.168.1.10, and one unparsable literal

**`process-readiness`** — tasks 1.2, 1.3, 1.4, 1.5
- states: `probe-unset`, `waiting-ready`, `ready`, `exited-before-ready`, `ready-timeout`
- transitions:
  - `start_with_connection on a manager with ready_probe == None` → `probe-unset` → **no-op** — anchor '    async fn failed_respawn_is_retried() {' — an existing test with no probe must stay green: wait_ready returns Ok immediately and spawn still means Running
  - `launch() returned Ok, probe address accepts a TCP connection, and child.try_wait() is still None at or after STABILITY_WINDOW from spawn` → `ready` → **set** — requirement sentence: 'its local SOCKS (or mixed) inbound accepts a TCP connection and the process is still running at least one second after it was spawned' — only here does start_with_connection run `self.state.transition(ProcessState::Running, connection)?`
  - `probe address accepts a TCP connection but child.try_wait() yields Some(status) before STABILITY_WINDOW elapsed` → `exited-before-ready` → **forced** — design decision: 'Port alone passes a sing-box that binds inbounds and then FATALs in post-start' — the accept does not win; the exit does
  - `child.try_wait() yields Some(status) at any point during the wait` → `exited-before-ready` → **forced** — anchor '        let mut msg = match exit_code {' — wait_ready calls cleanup_after_exit(), then builds the identical `process exited with code {code}: {last_output_line}` text (`process killed by signal` when code is None), then write_exit_record(false, Some(&status)), then returns Err(ProcessError::ExitedBeforeReady(msg))
  - `ready_timeout elapses with the child alive and no successful connect` → `ready-timeout` → **forced** — requirement sentence: 'When the backend does not become ready within 15 seconds, the system SHALL stop it and fail the start with an error naming the probed address and the timeout' — wait_ready calls graceful_stop() (which writes requested=true) and returns Err(ProcessError::ReadyTimeout { addr, timeout })
  - `wait_ready returned Err inside start_with_connection (initial start)` → `exited-before-ready | ready-timeout` → **forced** — anchor '        match self.launch().await {' — the Err arm's shape is reused: transition to ProcessState::Error(e.to_string()) and return Err(e); record_crash() is NOT called and no respawn loop runs
  - `wait_ready returned Err inside respawn()` → `exited-before-ready | ready-timeout` → **forced** — anchor '                Err(e) => {' in handle_unexpected_exit — the Err propagates out of respawn so the existing loop sets msg = `restart failed: {e}` and calls self.record_crash(), retrying while crash_times.len() < MAX_CRASHES
  - `xray TUN start: device wait and xray_up both succeeded inside launch()` → `waiting-ready` → **set** — anchor '            let run = tun::xray_up(&rt).await;' — wait_ready is invoked by start_with_connection/respawn after launch() returns Ok, never inside launch() before the helper block
  - `the caller drops the start_with_connection future while wait_ready is polling (Disconnect during Starting)` → `waiting-ready` → **no-op** — anchor '                Some(ConnectionCmd::Stop) = cmd_rx.recv() => {' in crates/ui/src/connection.rs — the manager writes nothing on cancel; mgr.shutdown() owns the teardown and the requested=true exit record
- forbidden:
  - record_crash() called for a readiness failure on the initial start — task 1.3 asserts crashes_in_window=0 in the exit record
  - routing a self-exited-before-ready child through graceful_stop: with the child already reaped by wait_ready the record would read requested=true and mislabel a startup failure as a user stop
  - two ' session ' records in backend.log for one candidate's initial start
  - a `reason=` field in the exit record — the exit-reason vocabulary (start-failed / crash / requested) belongs to log-connection-decisions (#3) and lands after this change; #1 writes only requested=false plus the existing code=/signal=, crashes_in_window=, last_output= fields
  - calling wait_ready from inside launch() — it would run before the xray device wait and xray_up
  - ProcessError::ExitedBeforeReady or ProcessError::ReadyTimeout classified as host-level: a startup failure is per-candidate and must let failover continue
  - reading last_output_line() before cleanup_after_exit() has drained the readers — the reason would be a stale snapshot
  - polling the probe faster than READY_POLL_INTERVAL or logging a line per poll — a crash-looping backend would flood the very log this sprint is making legible
  - A readiness TIMEOUT routes through graceful_stop and therefore writes `exit requested=true` with no reason field. That is correct today (the app did request the stop) and is NOT the exit-before-ready branch; log-connection-decisions gives it `reason=start-failed`. No test in this change asserts the timeout's exit record.
- seeding:
  - ready: manager_for(&dir, "exec sleep 30\n") + mgr.ready_probe set through with_ready_probe to the addr of a `std::net::TcpListener::bind("127.0.0.1:0")` the test keeps alive for the whole test body (do NOT drop it, unlike the bind-then-drop trick in crates/process/src/probe.rs), then mgr.start().await
  - exited-before-ready: manager_for(&dir, "echo FATAL >&2\nexit 1\n") with a live listener and a ready probe, then mgr.start().await — never by killing the child from the test and never by assigning mgr.child
  - ready-timeout: manager_for(&dir, "exec sleep 30\n") with a probe address taken from a listener that is bound and then dropped (the crates/process/src/probe.rs `fake_sleeper_binary` pairing) plus `mgr.ready_timeout = Duration::from_millis(300);` — the field is private and set directly, exactly as `mgr.restart_delay = Duration::from_millis(50);` already is
  - respawn readiness failure: a crashing_backend-shaped stub that serves once (exec sleep 30) then exits immediately on every later run, restart_delay 50 ms, a live listener, and repeated mgr.wait_and_handle_exit() until the state leaves Running
  - probe-unset: any existing test untouched — no with_ready_probe call at all
  - state is never seeded by calling self.state.transition directly from a test
- budgets:
  - READY_TIMEOUT: Duration = Duration::from_secs(15) — the default ready_timeout
  - STABILITY_WINDOW: Duration = Duration::from_secs(1) — measured from wait_ready entry (after launch(), so after the xray TUN device wait and xray-up), not from the first accept — the stricter variant, and it needs no extra field
  - READY_POLL_INTERVAL: Duration = Duration::from_millis(100) — fixed interval, no backoff, so the 15 s deadline is at most 150 polls
  - in-crate test override: ready_timeout 300 ms for the timeout case
  - restart_delay 50 ms in respawn tests; MAX_CRASHES 3 within CRASH_WINDOW 60 s unchanged
  - task 1.3 asserts no respawn within 3 s of the startup failure
  - wait_for_lines polls for at most 2 s; LOG_DRAIN_TIMEOUT 500 ms unchanged
  - one TCP connection per launch attempt, at most MAX_CRASHES+1 per session

**`ui-connection-ready-probe`** — tasks 2.1, 2.2
- states: `candidate-started`, `candidate-ready`, `candidate-failed-before-ready`, `stop-queued`, `forwarders-live`, `forwarders-halted`
- transitions:
  - `a candidate is built` → `candidate-started` → **set** — anchor '                .with_log_file(backend_log.clone()),' — `.with_ready_probe(effective_settings.local_endpoint(effective_settings.socks_port))` is appended to the same chain, inside configure(), per candidate
  - `resolve_effective_config overrode socks_port or listen_address for this node` → `candidate-started` → **forced** — anchor '            let (mut effective_rules, mut effective_settings) = resolve_effective_config(' — the probe address is read from effective_settings, never from the outer `settings`, because the generated config binds the effective values
  - `start_with_connection returns Ok (the backend is ready)` → `candidate-ready` → **set** — anchor '                    report(ProcessState::Running, Some(meta.clone()));' — unchanged code, but Ok now means ready; the queued-Stop check above it still runs first
  - `start_with_connection returns Err(ExitedBeforeReady) or Err(ReadyTimeout)` → `candidate-failed-before-ready` → **forced** — anchor '                    failures.push(CandidateFailure::new(' — is_host_level() is false for both variants, so the existing per-candidate Err branch parks the manager and moves to the next candidate with no crash-restart delay
  - `Stop arrives inside the supervision loop while forwarders are live` → `forwarders-halted` → **forced** — anchors '                        mgr.shutdown().await;' / '                        halt(state_forwarder).await;' / '                        report(ProcessState::Stopped, None);' — both this arm and the `_ =>` arm must halt log_forwarder as well before their terminal report
  - `the manager reaches a non-Running, non-Error state after wait_and_handle_exit` → `forwarders-halted` → **forced** — anchor '                            _ => {' — same halt set as the Stop arm
- forbidden:
  - reading the probe address from the outer `settings` instead of `effective_settings`
  - a UI test that expects ProcessState::Running without a live listener bound to its settings.socks_port — ready_timeout is a private process-crate field the ui crate cannot shorten, so such a test would burn the full 15 s and push RECV_TIMEOUT (20 s) to the edge
  - binding the listener and dropping it before the connection task runs
  - reporting Running for a candidate whose stub exits right after start, even when a shared listener makes the port accept
  - returning from an exit path with a forwarder still able to emit — assert_nothing_after_terminal exists precisely to catch it
- seeding:
  - candidate-ready: in the test, `let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();` kept alive for the test body, `settings.socks_port = listener.local_addr().unwrap().port();` (listen_address stays the default "127.0.0.1"), then connect(&stub, settings, candidates) with a stub whose run branch is `exec sleep 30`
  - candidate-failed-before-ready: same listener, a stub whose run branch prints FATAL to stderr and exits 1 for the matching candidate (the existing `grep -q <address> "$3"` shape selects which candidate fails)
  - stop-queued: handle.stop() after the Running report, as failover_reports_no_stopped_and_stop_reports_one already does
  - state is only ever observed through next_state / drain / assert_nothing_after_terminal; never by inspecting the manager
- budgets:
  - RECV_TIMEOUT 20 s per message, unchanged
  - +1 s (STABILITY_WINDOW) per candidate that reaches Running: failover_reports_no_stopped_and_stop_reports_one, log_lines_carry_connection_generation, live_connect_writes_backend_diagnostics, and the new two-candidate test
  - a candidate that exits before ready costs one launch, no 2 s/4 s crash-restart delays
  - exactly one ' session ' record per candidate attempt

**`ui-app-last-success`** — tasks 2.3
- states: `displayed-only`, `persisted`
- transitions:
  - `ProcessStateConnection(gen, ProcessState::Running, Some(meta)) for the current generation` → `persisted` → **set** — anchor '                        let settings = last_success_settings(&self.settings, meta);' — the only surviving persist path; requirement sentence: 'The last-success record SHALL change only when a connection succeeds, meaning the connection is reported Running after its backend became ready.'
  - `ProcessStateConnection(gen, ProcessState::Starting, Some(meta)) — the respawn relay` → `displayed-only` → **no-op** — requirement sentence: 'A connection that reaches only Starting, or whose backend exits or times out before it is ready, SHALL NOT change it.'; the relay source is `fn relays(state: &ProcessState) -> bool` in crates/ui/src/connection.rs
  - `ProcessStateConnection(gen, ProcessState::Stopping, Some(meta))` → `displayed-only` → **no-op** — anchor '                if connection.is_some() {' — self.connection_status is still assigned; only the persist below it is gated
  - `ProcessStateConnection(gen, ProcessState::Running, None)` → `displayed-only` → **no-op** — anchor '                } else if matches!(state, ProcessState::Stopped | ProcessState::Error(_)) {' — with no metadata there is nothing to record; records_last_success returns false
  - `ProcessStateConnection for a superseded generation` → `displayed-only` → **no-op** — anchor '                if !is_current_generation(generation, self.connection_generation) {' — the existing guard returns before any of this; unchanged
  - `a settings flush from the preferences dialog` → `displayed-only` → **no-op** — anchor '                let settings = keep_last_success(settings, &self.settings);' — unchanged; requirement sentence: 'Saving settings from the preferences dialog SHALL NOT modify it.'
- forbidden:
  - clearing an existing last_success when a start fails before ready — the record keeps naming the previously successful node (spec scenario 'Backend dies before ready')
  - gating self.connection_status on records_last_success: the status bar must still follow Starting and Stopping relays
  - persisting settings on any state other than Running
- seeding:
  - the pure fn is called directly with constructed ProcessState values and an Option<&ConnectionMetadata> built like the one in direct_session_success_records_last_success; no App instance, no GTK, no persistence — app.rs tests are otherwise pure
- budgets:
  - 3 assertion rows: Starting+meta false, Running+meta true, Running+None false
  - at most one persist_settings call per Running report

**`verification`** — tasks 3.1, 3.2
- states: `automated-floor`, `manual-live`
- transitions:
  - `the full workspace suite` → `automated-floor` → **set** — Makefile anchor 'TEST := timeout $(TEST_TIMEOUT) $(CARGO) test' with TEST_ARGS '-- --test-threads=$(TEST_THREADS)'
  - `sing-box TUN on an IPv6-less host` → `manual-live` → **no-op** — tasks.md 3.2 — status never shows Connected, one ' session ' per candidate with no 2 s/4 s respawns, settings.toml last_success unchanged
  - `xray TUN on a working node` → `manual-live` → **set** — tasks.md 3.2 — Connected only after routes are up, last_success updated
- forbidden:
  - a bare `cargo test` without the timeout and --test-threads cap
  - treating 3.2 as done from the automated suite
- seeding:
  - automated-floor: `make test` from the repo root (TEST_TIMEOUT 5m, TEST_THREADS 4 are Makefile defaults; raise with `make test TEST_TIMEOUT=10m` for the workspace run)
  - manual-live: the operator runs the app against an installed backend; report the backend.log excerpt and the settings.toml last_success value
- budgets:
  - workspace run bounded at 10m, --test-threads=4
  - per-crate runs bounded at 5m

### Floor

make test TEST_TIMEOUT=10m (timeout 10m cargo test --workspace --all-targets -- --test-threads=4) and make lint (cargo fmt -- --check; cargo clippy --workspace --all-targets --all-features -- -D warnings), both green; plus the two manual live checks of task 3.2.


### Requirements map

- The last-success record SHALL change only when a connection succeeds, meaning the connection is reported `Running` after its backend became ready. A connection that reaches only `Starting`, or whose b…
  - tests: `crates/ui/src/app.rs::tests::records_last_success_only_on_running`; `crates/ui/src/connection.rs::tests::two_candidates_first_exits_before_ready`
- - **THEN** the persisted last-success record SHALL still name node B
  - tests: `crates/ui/src/app.rs::tests::records_last_success_only_on_running`
- - **THEN** the persisted last-success record SHALL still name node A
  - tests: `crates/ui/src/app.rs::tests::records_last_success_only_on_running`
- - **THEN** the last-success record SHALL change only when the respawned backend is reported `Running`, and SHALL NOT change on the intermediate `Starting`
  - tests: `crates/ui/src/app.rs::tests::records_last_success_only_on_running`
- The system SHALL report a backend start, and an in-place respawn, as `Running` only after the backend is ready: its local SOCKS (or mixed) inbound accepts a TCP connection and the process is still run…
  - tests: `crates/process/src/manager.rs::tests::ready_probe_accepts_live_backend`; `crates/process/src/manager.rs::tests::ready_timeout_stops_the_backend`; `crates/process/src/manager.rs::tests::exit_before_ready_carries_last_output`; `crates/core/src/models/settings.rs::tests::local_endpoint_maps_unspecified_to_loopback`
- - **THEN** the connection SHALL NOT be reported `Running`, no respawn SHALL be attempted for that candidate, and the start SHALL fail with a reason containing the exit code and the last output line
  - tests: `crates/process/src/manager.rs::tests::startup_failure_writes_one_session_and_no_crash`; `crates/process/src/manager.rs::tests::exit_before_ready_carries_last_output`
- - **THEN** the system SHALL report `Running` with the connection metadata
  - tests: `crates/process/src/manager.rs::tests::ready_probe_accepts_live_backend`; `crates/ui/src/connection.rs::tests::live_connect_writes_backend_diagnostics`
- - **THEN** the system SHALL stop the backend and fail the start with an error naming the address and the timeout
  - tests: `crates/process/src/manager.rs::tests::ready_timeout_stops_the_backend`
- - **THEN** the connection SHALL try the next candidate without waiting for a crash-restart delay
  - tests: `crates/ui/src/connection.rs::tests::two_candidates_first_exits_before_ready`; `crates/ui/src/connection.rs::tests::last_candidate_failure_reports_one_error`
- - **THEN** the system SHALL wait for the TUN device and run the route helper before checking readiness, and SHALL report `Running` only after all three succeed
  - tests: `MANUAL task 3.2-m2: xray TUN on a working node — Connected only after the device wait and xray-up succeed`
- - **THEN** the system SHALL stop the backend, release TUN routing state, and report `Stopped`
  - tests: `crates/process/src/manager.rs::tests::ready_timeout_stops_the_backend`; `crates/process/src/manager.rs::tests::failed_respawn_is_retried`; `crates/ui/src/connection.rs::tests::stop_halts_every_forwarder`
- The system SHALL detect unexpected exits of a backend that has been reported `Running` and restart it automatically within a bounded budget. An exit before the backend was first ready is a startup fai…
  - tests: `crates/process/src/manager.rs::tests::respawn_readiness_failure_exhausts_crash_budget`; `crates/process/src/manager.rs::tests::startup_failure_writes_one_session_and_no_crash`; `crates/process/src/manager.rs::tests::respawn_skips_preflight`
- - **THEN** the system SHALL wait 2 seconds and attempt to restart automatically
  - tests: `crates/process/src/manager.rs::tests::failed_respawn_is_retried`
- - **THEN** the system SHALL transition to Error state instead of restarting. Any exit while the backend is expected to be running counts as a crash, including a signal death (OOM, segfault, external k…
  - tests: `crates/process/src/manager.rs::tests::respawn_readiness_failure_exhausts_crash_budget`; `crates/process/src/manager.rs::tests::respawn_budget_exhaustion_errors`
- - **THEN** the system SHALL NOT wait, SHALL NOT respawn, SHALL NOT count a crash, and SHALL fail the start with the exit reason
  - tests: `crates/process/src/manager.rs::tests::startup_failure_writes_one_session_and_no_crash`
- - **THEN** the failure SHALL be recorded as a crash and the system SHALL wait and respawn again instead of transitioning to Error
  - tests: `crates/process/src/manager.rs::tests::respawn_readiness_failure_exhausts_crash_budget`
- - **THEN** it SHALL relaunch without re-running the backend version probe, the capability probe, or the backend's config check
  - tests: `crates/process/src/manager.rs::tests::respawn_skips_preflight`
- - **THEN** the policy rules installed for the session SHALL remain in place from the exit until the respawned backend's routes are programmed
  - tests: `MANUAL task 3.2-m2: xray TUN respawn keeps the session's policy rules`

### Plan review

`pass` by zarchitect, 2 rounds. Round 1 raised three blockers, all fixed: r1's sites named `crates/process/src/manager.rs` while its codeTasks, packages and verify named `crates/core`; r5 carried two sites for a `ready_probe_addr` symbol no chunk creates; and r5 compiled `local_endpoint` with no edge to the chunk that adds it. Round 2 confirmed them closed and added two site-level alignments.

Carried warnings, not defects:

- The xray-TUN ordering requirement (device wait and route helper before readiness) is covered by the manual live step only; nothing automated guards against `wait_ready` being called inside `launch()`.
- `stop_halts_every_forwarder` covers the forwarder-halt clause of the Stopped requirement, not the TUN teardown and route release clause.
- The probe opens and drops one TCP connection per launch attempt; both backends log the aborted handshake.

## Plan appendix

```json
{
  "v": 2,
  "change": "confirm-backend-ready-before-running",
  "baseSha": "1d6a0fb8eb02e73a65fc393adfdd743e2bba742f",
  "generatedAt": "2026-09-16T09:09:20.051084+00:00",
  "tier": "heavy",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality",
    "arch"
  ],
  "chunks": [
    {
      "id": "r1",
      "taskIds": [
        "1.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "core-local-endpoint",
      "shard": "core",
      "pkgDirs": [
        "crates/core/src/models"
      ],
      "pkgs": [
        "v2ray-rs-core"
      ],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/models/settings.rs",
          "symbol": "AppSettings::local_endpoint (new)",
          "anchor": "pub fn validate_listen_address(addr: &str) -> Result<(), ValidationError> {",
          "change": "Add `pub fn local_endpoint(&self, port: u16) -> SocketAddr` to the same impl block; unspecified v4/v6 maps to the loopback of that family, parse failure falls back to Ipv4Addr::LOCALHOST."
        },
        {
          "task": "1.1",
          "file": "crates/core/src/models/settings.rs",
          "symbol": "tests::local_endpoint_maps_unspecified_to_loopback (new)",
          "anchor": "fn test_validate_listen_address()",
          "change": "Add the sibling unit test asserting the five rows: 127.0.0.1, 0.0.0.0, ::, 192.168.1.10, and an unparsable literal."
        }
      ],
      "contract": {
        "states": [
          "loopback-v4",
          "unspecified-v4",
          "unspecified-v6",
          "explicit-v4",
          "unparsable"
        ],
        "transitions": [
          {
            "input": "listen_address \"127.0.0.1\", port 1080",
            "state": "loopback-v4",
            "effect": "no-op",
            "evidence": "anchor 'pub fn validate_listen_address(addr: &str) -> Result<(), ValidationError> {' — the literal is already a probeable address; local_endpoint returns 127.0.0.1:1080"
          },
          {
            "input": "listen_address \"0.0.0.0\", port 1080",
            "state": "unspecified-v4",
            "effect": "forced",
            "evidence": "requirement sentence: 'An unspecified listen address SHALL be probed on the loopback address of the same family.' — forced to 127.0.0.1:1080"
          },
          {
            "input": "listen_address \"::\", port 1080",
            "state": "unspecified-v6",
            "effect": "forced",
            "evidence": "requirement sentence: 'An unspecified listen address SHALL be probed on the loopback address of the same family.' — forced to [::1]:1080"
          },
          {
            "input": "listen_address \"192.168.1.10\", port 1080",
            "state": "explicit-v4",
            "effect": "no-op",
            "evidence": "anchor '        let valid = [\"127.0.0.1\", \"0.0.0.0\", \"::\", \"::1\", \"192.168.1.10\"];' — an explicit bind address is probed as written"
          },
          {
            "input": "listen_address that IpAddr::from_str rejects (\"\", \"localhost\")",
            "state": "unparsable",
            "effect": "forced",
            "evidence": "anchor '        IpAddr::from_str(addr)' — validate_listen_address rejects these at edit time, but a loaded settings.toml can still carry one; local_endpoint forces 127.0.0.1:<port> instead of panicking or unwrapping"
          }
        ],
        "forbidden": [
          "local_endpoint returning an unspecified address (0.0.0.0 or ::) — nothing can TCP-connect to it",
          "local_endpoint panicking or returning Result — every caller sits on a start path and has no recovery to offer",
          "a second address formatter: crates/ui/src/connection.rs keeps `fn listen_endpoint(addr: &str, port: u16) -> String` for the human-facing v2ray warning text (bracketed display form), and it is NOT replaced by local_endpoint in this change"
        ],
        "seeding": [
          "every state is reached by constructing AppSettings::default() and assigning settings.listen_address plus the port argument; no file I/O, no persistence"
        ],
        "budgets": [
          "pure function, no await, no syscall — 0 ms of wall clock",
          "5 assertion cases: 127.0.0.1, 0.0.0.0, ::, 192.168.1.10, and one unparsable literal"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.1-t1: in crates/core/src/models/settings.rs `mod tests`, next to `fn test_validate_listen_address()`, add `fn local_endpoint_maps_unspecified_to_loopback()` asserting the five rows of the transition table as SocketAddr values (127.0.0.1:1080, 127.0.0.1:1080, [::1]:1080, 192.168.1.10:1080, 127.0.0.1:1080)",
        "1.1-t2: add `pub fn local_endpoint(&self, port: u16) -> SocketAddr` to the `impl AppSettings` block that holds validate_listen_address; parse self.listen_address with IpAddr::from_str (already imported via `use std::str::FromStr;` and `use std::net::IpAddr;`), map IpAddr::is_unspecified to Ipv4Addr::LOCALHOST / Ipv6Addr::LOCALHOST of the same family, and fall back to Ipv4Addr::LOCALHOST on a parse error; extend the `use std::net::` line to bring in SocketAddr, Ipv4Addr, Ipv6Addr",
        "1.1-t3: `make test-core` green"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-core && cargo clippy -p v2ray-rs-core --all-targets -- -D warnings",
      "coder": "rust-coder"
    },
    {
      "id": "r2",
      "taskIds": [
        "1.2"
      ],
      "prev": "r1",
      "sharedPkg": null,
      "parallel": true,
      "seam": "process-readiness",
      "shard": "process",
      "pkgDirs": [
        "crates/process/src"
      ],
      "pkgs": [
        "v2ray-rs-process"
      ],
      "sites": [
        {
          "task": "1.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessError",
          "anchor": "    #[error(\"config rejected by backend: {0}\")]",
          "change": "add variants: ExitedBeforeReady(String) and a readiness-timeout variant naming address + duration; both non-host-level"
        },
        {
          "task": "1.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager fields",
          "anchor": "    restart_delay: Duration,",
          "change": "add ready_probe: Option<SocketAddr> and ready_timeout: Duration (test-settable, same shape as restart_delay)"
        },
        {
          "task": "1.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::new",
          "anchor": "            restart_delay: CRASH_RESTART_DELAY,",
          "change": "initialise ready_probe: None and ready_timeout: READY_TIMEOUT (15 s)"
        },
        {
          "task": "1.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::with_ready_probe",
          "anchor": "    pub fn with_tun(mut self, tun: Option<TunRuntime>) -> Self {",
          "change": "add builder with_ready_probe(SocketAddr) next to with_tun"
        },
        {
          "task": "1.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::wait_ready (new)",
          "anchor": "    // A launch failure leaves routing state in place: during a respawn the",
          "change": "add async wait_ready() after launch(): poll TCP connect every 100 ms, require child alive STABILITY_WINDOW (1 s) after spawn; on child exit build the reason like handle_unexpected_exit and return ExitedBeforeReady; on timeout return the timeout error; no-op Ok when ready_probe is None"
        },
        {
          "task": "1.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "exit reason formatting",
          "anchor": "        let mut msg = match exit_code {",
          "change": "reuse/extract this `process exited with code N: <last line>` shape so ExitedBeforeReady carries the identical text (shared-failure key normalisation depends on it)"
        },
        {
          "task": "1.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "mod tests (wait_ready)",
          "anchor": "fn manager_for(dir: &tempfile::TempDir, script_body: &str) -> ProcessManager {",
          "change": "add tests using manager_for + a test-owned TcpListener: `exec sleep 30` + listener -> ready; `echo FATAL; exit 1` -> ExitedBeforeReady containing FATAL; `exec sleep 30` with no listener and 300 ms ready_timeout -> timeout error and child reaped"
        }
      ],
      "contract": {
        "states": [
          "probe-unset",
          "waiting-ready",
          "ready",
          "exited-before-ready",
          "ready-timeout"
        ],
        "transitions": [
          {
            "input": "start_with_connection on a manager with ready_probe == None",
            "state": "probe-unset",
            "effect": "no-op",
            "evidence": "anchor '    async fn failed_respawn_is_retried() {' — an existing test with no probe must stay green: wait_ready returns Ok immediately and spawn still means Running"
          },
          {
            "input": "launch() returned Ok, probe address accepts a TCP connection, and child.try_wait() is still None at or after STABILITY_WINDOW from spawn",
            "state": "ready",
            "effect": "set",
            "evidence": "requirement sentence: 'its local SOCKS (or mixed) inbound accepts a TCP connection and the process is still running at least one second after it was spawned' — only here does start_with_connection run `self.state.transition(ProcessState::Running, connection)?`"
          },
          {
            "input": "probe address accepts a TCP connection but child.try_wait() yields Some(status) before STABILITY_WINDOW elapsed",
            "state": "exited-before-ready",
            "effect": "forced",
            "evidence": "design decision: 'Port alone passes a sing-box that binds inbounds and then FATALs in post-start' — the accept does not win; the exit does"
          },
          {
            "input": "child.try_wait() yields Some(status) at any point during the wait",
            "state": "exited-before-ready",
            "effect": "forced",
            "evidence": "anchor '        let mut msg = match exit_code {' — wait_ready calls cleanup_after_exit(), then builds the identical `process exited with code {code}: {last_output_line}` text (`process killed by signal` when code is None), then write_exit_record(false, Some(&status)), then returns Err(ProcessError::ExitedBeforeReady(msg))"
          },
          {
            "input": "ready_timeout elapses with the child alive and no successful connect",
            "state": "ready-timeout",
            "effect": "forced",
            "evidence": "requirement sentence: 'When the backend does not become ready within 15 seconds, the system SHALL stop it and fail the start with an error naming the probed address and the timeout' — wait_ready calls graceful_stop() (which writes requested=true) and returns Err(ProcessError::ReadyTimeout { addr, timeout })"
          },
          {
            "input": "wait_ready returned Err inside start_with_connection (initial start)",
            "state": "exited-before-ready | ready-timeout",
            "effect": "forced",
            "evidence": "anchor '        match self.launch().await {' — the Err arm's shape is reused: transition to ProcessState::Error(e.to_string()) and return Err(e); record_crash() is NOT called and no respawn loop runs"
          },
          {
            "input": "wait_ready returned Err inside respawn()",
            "state": "exited-before-ready | ready-timeout",
            "effect": "forced",
            "evidence": "anchor '                Err(e) => {' in handle_unexpected_exit — the Err propagates out of respawn so the existing loop sets msg = `restart failed: {e}` and calls self.record_crash(), retrying while crash_times.len() < MAX_CRASHES"
          },
          {
            "input": "xray TUN start: device wait and xray_up both succeeded inside launch()",
            "state": "waiting-ready",
            "effect": "set",
            "evidence": "anchor '            let run = tun::xray_up(&rt).await;' — wait_ready is invoked by start_with_connection/respawn after launch() returns Ok, never inside launch() before the helper block"
          },
          {
            "input": "the caller drops the start_with_connection future while wait_ready is polling (Disconnect during Starting)",
            "state": "waiting-ready",
            "effect": "no-op",
            "evidence": "anchor '                Some(ConnectionCmd::Stop) = cmd_rx.recv() => {' in crates/ui/src/connection.rs — the manager writes nothing on cancel; mgr.shutdown() owns the teardown and the requested=true exit record"
          }
        ],
        "forbidden": [
          "record_crash() called for a readiness failure on the initial start — task 1.3 asserts crashes_in_window=0 in the exit record",
          "routing a self-exited-before-ready child through graceful_stop: with the child already reaped by wait_ready the record would read requested=true and mislabel a startup failure as a user stop",
          "two ' session ' records in backend.log for one candidate's initial start",
          "a `reason=` field in the exit record — the exit-reason vocabulary (start-failed / crash / requested) belongs to log-connection-decisions (#3) and lands after this change; #1 writes only requested=false plus the existing code=/signal=, crashes_in_window=, last_output= fields",
          "calling wait_ready from inside launch() — it would run before the xray device wait and xray_up",
          "ProcessError::ExitedBeforeReady or ProcessError::ReadyTimeout classified as host-level: a startup failure is per-candidate and must let failover continue",
          "reading last_output_line() before cleanup_after_exit() has drained the readers — the reason would be a stale snapshot",
          "polling the probe faster than READY_POLL_INTERVAL or logging a line per poll — a crash-looping backend would flood the very log this sprint is making legible",
          "A readiness TIMEOUT routes through graceful_stop and therefore writes `exit requested=true` with no reason field. That is correct today (the app did request the stop) and is NOT the exit-before-ready branch; log-connection-decisions gives it `reason=start-failed`. No test in this change asserts the timeout's exit record."
        ],
        "seeding": [
          "ready: manager_for(&dir, \"exec sleep 30\\n\") + mgr.ready_probe set through with_ready_probe to the addr of a `std::net::TcpListener::bind(\"127.0.0.1:0\")` the test keeps alive for the whole test body (do NOT drop it, unlike the bind-then-drop trick in crates/process/src/probe.rs), then mgr.start().await",
          "exited-before-ready: manager_for(&dir, \"echo FATAL >&2\\nexit 1\\n\") with a live listener and a ready probe, then mgr.start().await — never by killing the child from the test and never by assigning mgr.child",
          "ready-timeout: manager_for(&dir, \"exec sleep 30\\n\") with a probe address taken from a listener that is bound and then dropped (the crates/process/src/probe.rs `fake_sleeper_binary` pairing) plus `mgr.ready_timeout = Duration::from_millis(300);` — the field is private and set directly, exactly as `mgr.restart_delay = Duration::from_millis(50);` already is",
          "respawn readiness failure: a crashing_backend-shaped stub that serves once (exec sleep 30) then exits immediately on every later run, restart_delay 50 ms, a live listener, and repeated mgr.wait_and_handle_exit() until the state leaves Running",
          "probe-unset: any existing test untouched — no with_ready_probe call at all",
          "state is never seeded by calling self.state.transition directly from a test"
        ],
        "budgets": [
          "READY_TIMEOUT: Duration = Duration::from_secs(15) — the default ready_timeout",
          "STABILITY_WINDOW: Duration = Duration::from_secs(1) — measured from wait_ready entry (after launch(), so after the xray TUN device wait and xray-up), not from the first accept — the stricter variant, and it needs no extra field",
          "READY_POLL_INTERVAL: Duration = Duration::from_millis(100) — fixed interval, no backoff, so the 15 s deadline is at most 150 polls",
          "in-crate test override: ready_timeout 300 ms for the timeout case",
          "restart_delay 50 ms in respawn tests; MAX_CRASHES 3 within CRASH_WINDOW 60 s unchanged",
          "task 1.3 asserts no respawn within 3 s of the startup failure",
          "wait_for_lines polls for at most 2 s; LOG_DRAIN_TIMEOUT 500 ms unchanged",
          "one TCP connection per launch attempt, at most MAX_CRASHES+1 per session"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.2-t1: test `ready_probe_accepts_live_backend` — listener alive, `exec sleep 30` stub, with_ready_probe(listener addr); start() -> Ok, mgr.state() == ProcessState::Running; assert the start took at least STABILITY_WINDOW",
        "1.2-t2: test `exit_before_ready_carries_last_output` — stub `echo FATAL >&2` then `exit 1`, live listener, with_ready_probe; start() -> Err(ProcessError::ExitedBeforeReady(msg)) with msg containing \"code 1\" and \"FATAL\"",
        "1.2-t3: test `ready_timeout_stops_the_backend` — `exec sleep 30` stub, probe address from a bound-then-dropped listener, mgr.ready_timeout = 300 ms; start() -> Err(ProcessError::ReadyTimeout { .. }) whose Display names the address and `300ms`; mgr.child.is_none() afterwards (the child was reaped by graceful_stop)",
        "1.2-c1: add `const READY_TIMEOUT: Duration = Duration::from_secs(15);`, `const STABILITY_WINDOW: Duration = Duration::from_secs(1);` and `const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);` beside the anchor 'const CRASH_RESTART_DELAY: Duration = Duration::from_secs(2);'",
        "1.2-c2: add to the ProcessError enum, after the anchor '    #[error(\"config rejected by backend: {0}\")]' variant: `#[error(\"{0}\")] ExitedBeforeReady(String)` and `#[error(\"backend did not accept connections on {addr} within {timeout:?}\")] ReadyTimeout { addr: SocketAddr, timeout: Duration }`; import std::net::SocketAddr",
        "1.2-c3: extend the `is_host_level_classifies_every_variant` case table with both new variants mapped to false (the test enumerates every variant and is the project's standing guard)",
        "1.2-c4: add fields `ready_probe: Option<SocketAddr>` and `ready_timeout: Duration` beside the anchor '    restart_delay: Duration,'; initialise them to None and READY_TIMEOUT beside the anchor '            restart_delay: CRASH_RESTART_DELAY,'",
        "1.2-c5: add `pub fn with_ready_probe(mut self, addr: SocketAddr) -> Self` next to the anchor '    pub fn with_tun(mut self, tun: Option<TunRuntime>) -> Self {', following the consuming-builder pattern",
        "1.2-c6: extract the exit-reason text at the anchor '        let mut msg = match exit_code {' into a private helper (status + last_output_line -> String) and call it from both handle_unexpected_exit and wait_ready, so the ExitedBeforeReady text is identical to a crash reason and failure_key normalisation keeps working",
        "1.2-c7: add `async fn wait_ready(&mut self) -> Result<(), ProcessError>` after the anchor '    // A launch failure leaves routing state in place: during a respawn the' — Ok(()) at once when ready_probe is None; otherwise record `probe_start: Instant` at wait_ready entry, loop every READY_POLL_INTERVAL: child.try_wait() Some(status) -> cleanup_after_exit().await, write_exit_record(false, Some(&status)), Err(ExitedBeforeReady(reason)); tokio::time::timeout(READY_POLL_INTERVAL, TcpStream::connect(addr)) Ok and probe_start.elapsed() >= STABILITY_WINDOW -> Ok(()) — an unbounded connect to a non-loopback address behind a drop rule outlives ready_timeout and the 15 s budget stops being enforced; deadline passed -> graceful_stop().await, Err(ReadyTimeout { addr, timeout: self.ready_timeout })"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-process && cargo clippy -p v2ray-rs-process --all-targets -- -D warnings",
      "coder": "rust-coder"
    },
    {
      "id": "r3",
      "taskIds": [
        "1.3"
      ],
      "prev": "r2",
      "sharedPkg": "crates/process/src",
      "parallel": false,
      "seam": "process-readiness",
      "shard": "process",
      "pkgDirs": [
        "crates/process/src"
      ],
      "pkgs": [
        "v2ray-rs-process"
      ],
      "sites": [
        {
          "task": "1.3",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::start_with_connection",
          "anchor": "        match self.launch().await {",
          "change": "on Ok(()) call wait_ready() before transitioning to Running; on readiness failure graceful_stop the child, write the exit record without record_crash, transition Starting -> Error(e), return Err(e)"
        },
        {
          "task": "1.3",
          "file": "crates/process/src/manager.rs",
          "symbol": "mod tests (startup failure)",
          "anchor": "    async fn crash_error_includes_last_stderr_line() {",
          "change": "add test: FATAL-then-exit stub with a ready probe -> Err(ExitedBeforeReady), one ` session ` record in backend.log, crashes_in_window=0 in the exit record, no respawn after 3 s"
        }
      ],
      "contract": {
        "states": [
          "probe-unset",
          "waiting-ready",
          "ready",
          "exited-before-ready",
          "ready-timeout"
        ],
        "transitions": [
          {
            "input": "start_with_connection on a manager with ready_probe == None",
            "state": "probe-unset",
            "effect": "no-op",
            "evidence": "anchor '    async fn failed_respawn_is_retried() {' — an existing test with no probe must stay green: wait_ready returns Ok immediately and spawn still means Running"
          },
          {
            "input": "launch() returned Ok, probe address accepts a TCP connection, and child.try_wait() is still None at or after STABILITY_WINDOW from spawn",
            "state": "ready",
            "effect": "set",
            "evidence": "requirement sentence: 'its local SOCKS (or mixed) inbound accepts a TCP connection and the process is still running at least one second after it was spawned' — only here does start_with_connection run `self.state.transition(ProcessState::Running, connection)?`"
          },
          {
            "input": "probe address accepts a TCP connection but child.try_wait() yields Some(status) before STABILITY_WINDOW elapsed",
            "state": "exited-before-ready",
            "effect": "forced",
            "evidence": "design decision: 'Port alone passes a sing-box that binds inbounds and then FATALs in post-start' — the accept does not win; the exit does"
          },
          {
            "input": "child.try_wait() yields Some(status) at any point during the wait",
            "state": "exited-before-ready",
            "effect": "forced",
            "evidence": "anchor '        let mut msg = match exit_code {' — wait_ready calls cleanup_after_exit(), then builds the identical `process exited with code {code}: {last_output_line}` text (`process killed by signal` when code is None), then write_exit_record(false, Some(&status)), then returns Err(ProcessError::ExitedBeforeReady(msg))"
          },
          {
            "input": "ready_timeout elapses with the child alive and no successful connect",
            "state": "ready-timeout",
            "effect": "forced",
            "evidence": "requirement sentence: 'When the backend does not become ready within 15 seconds, the system SHALL stop it and fail the start with an error naming the probed address and the timeout' — wait_ready calls graceful_stop() (which writes requested=true) and returns Err(ProcessError::ReadyTimeout { addr, timeout })"
          },
          {
            "input": "wait_ready returned Err inside start_with_connection (initial start)",
            "state": "exited-before-ready | ready-timeout",
            "effect": "forced",
            "evidence": "anchor '        match self.launch().await {' — the Err arm's shape is reused: transition to ProcessState::Error(e.to_string()) and return Err(e); record_crash() is NOT called and no respawn loop runs"
          },
          {
            "input": "wait_ready returned Err inside respawn()",
            "state": "exited-before-ready | ready-timeout",
            "effect": "forced",
            "evidence": "anchor '                Err(e) => {' in handle_unexpected_exit — the Err propagates out of respawn so the existing loop sets msg = `restart failed: {e}` and calls self.record_crash(), retrying while crash_times.len() < MAX_CRASHES"
          },
          {
            "input": "xray TUN start: device wait and xray_up both succeeded inside launch()",
            "state": "waiting-ready",
            "effect": "set",
            "evidence": "anchor '            let run = tun::xray_up(&rt).await;' — wait_ready is invoked by start_with_connection/respawn after launch() returns Ok, never inside launch() before the helper block"
          },
          {
            "input": "the caller drops the start_with_connection future while wait_ready is polling (Disconnect during Starting)",
            "state": "waiting-ready",
            "effect": "no-op",
            "evidence": "anchor '                Some(ConnectionCmd::Stop) = cmd_rx.recv() => {' in crates/ui/src/connection.rs — the manager writes nothing on cancel; mgr.shutdown() owns the teardown and the requested=true exit record"
          }
        ],
        "forbidden": [
          "record_crash() called for a readiness failure on the initial start — task 1.3 asserts crashes_in_window=0 in the exit record",
          "routing a self-exited-before-ready child through graceful_stop: with the child already reaped by wait_ready the record would read requested=true and mislabel a startup failure as a user stop",
          "two ' session ' records in backend.log for one candidate's initial start",
          "a `reason=` field in the exit record — the exit-reason vocabulary (start-failed / crash / requested) belongs to log-connection-decisions (#3) and lands after this change; #1 writes only requested=false plus the existing code=/signal=, crashes_in_window=, last_output= fields",
          "calling wait_ready from inside launch() — it would run before the xray device wait and xray_up",
          "ProcessError::ExitedBeforeReady or ProcessError::ReadyTimeout classified as host-level: a startup failure is per-candidate and must let failover continue",
          "reading last_output_line() before cleanup_after_exit() has drained the readers — the reason would be a stale snapshot",
          "polling the probe faster than READY_POLL_INTERVAL or logging a line per poll — a crash-looping backend would flood the very log this sprint is making legible",
          "A readiness TIMEOUT routes through graceful_stop and therefore writes `exit requested=true` with no reason field. That is correct today (the app did request the stop) and is NOT the exit-before-ready branch; log-connection-decisions gives it `reason=start-failed`. No test in this change asserts the timeout's exit record."
        ],
        "seeding": [
          "ready: manager_for(&dir, \"exec sleep 30\\n\") + mgr.ready_probe set through with_ready_probe to the addr of a `std::net::TcpListener::bind(\"127.0.0.1:0\")` the test keeps alive for the whole test body (do NOT drop it, unlike the bind-then-drop trick in crates/process/src/probe.rs), then mgr.start().await",
          "exited-before-ready: manager_for(&dir, \"echo FATAL >&2\\nexit 1\\n\") with a live listener and a ready probe, then mgr.start().await — never by killing the child from the test and never by assigning mgr.child",
          "ready-timeout: manager_for(&dir, \"exec sleep 30\\n\") with a probe address taken from a listener that is bound and then dropped (the crates/process/src/probe.rs `fake_sleeper_binary` pairing) plus `mgr.ready_timeout = Duration::from_millis(300);` — the field is private and set directly, exactly as `mgr.restart_delay = Duration::from_millis(50);` already is",
          "respawn readiness failure: a crashing_backend-shaped stub that serves once (exec sleep 30) then exits immediately on every later run, restart_delay 50 ms, a live listener, and repeated mgr.wait_and_handle_exit() until the state leaves Running",
          "probe-unset: any existing test untouched — no with_ready_probe call at all",
          "state is never seeded by calling self.state.transition directly from a test"
        ],
        "budgets": [
          "READY_TIMEOUT: Duration = Duration::from_secs(15) — the default ready_timeout",
          "STABILITY_WINDOW: Duration = Duration::from_secs(1) — measured from wait_ready entry (after launch(), so after the xray TUN device wait and xray-up), not from the first accept — the stricter variant, and it needs no extra field",
          "READY_POLL_INTERVAL: Duration = Duration::from_millis(100) — fixed interval, no backoff, so the 15 s deadline is at most 150 polls",
          "in-crate test override: ready_timeout 300 ms for the timeout case",
          "restart_delay 50 ms in respawn tests; MAX_CRASHES 3 within CRASH_WINDOW 60 s unchanged",
          "task 1.3 asserts no respawn within 3 s of the startup failure",
          "wait_for_lines polls for at most 2 s; LOG_DRAIN_TIMEOUT 500 ms unchanged",
          "one TCP connection per launch attempt, at most MAX_CRASHES+1 per session"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.3-t1: test `startup_failure_writes_one_session_and_no_crash` — FATAL-then-exit stub with .with_log_file(Some(backend_log(dir.path()))) and a ready probe; start() -> Err(ExitedBeforeReady); backend.log holds exactly one \" session \" record; the exit record contains \"exit requested=false\", \"code=1\", \"crashes_in_window=0\" and \"last_output=FATAL\"; after sleeping 3 s the log still holds exactly one session record (no respawn)",
        "1.3-c1: in start_with_connection at the anchor '        match self.launch().await {', change the Ok(()) arm to `self.wait_ready().await` first and transition to Running only on its Ok; on Err reuse the existing Err arm shape — transition to ProcessState::Error(e.to_string()) and return Err(e), with no record_crash and no graceful_stop of an already-reaped child"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-process && cargo clippy -p v2ray-rs-process --all-targets -- -D warnings",
      "coder": "rust-coder"
    },
    {
      "id": "r4",
      "taskIds": [
        "1.4",
        "1.5"
      ],
      "prev": "r3",
      "sharedPkg": "crates/process/src",
      "parallel": false,
      "seam": "process-readiness",
      "shard": "process",
      "pkgDirs": [
        "crates/process/src"
      ],
      "pkgs": [
        "v2ray-rs-process"
      ],
      "sites": [
        {
          "task": "1.4",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::respawn",
          "anchor": "    async fn respawn(&mut self) -> Result<(), ProcessError> {",
          "change": "call wait_ready() after launch() and before the Running transition; its Err propagates so handle_unexpected_exit's loop records a crash and retries"
        },
        {
          "task": "1.4",
          "file": "crates/process/src/manager.rs",
          "symbol": "mod tests (respawn readiness)",
          "anchor": "    async fn respawn_budget_exhaustion_errors() {",
          "change": "add sibling test: stub serves once, crashes, then exits immediately on every respawn -> Error(\"3 crashes within 60s: …\") after two respawns"
        },
        {
          "task": "1.5",
          "file": "crates/process/src/manager.rs",
          "symbol": "mod tests (existing)",
          "anchor": "    async fn failed_respawn_is_retried() {",
          "change": "must stay green unchanged: no ready probe set -> wait_ready is a no-op and spawn still means Running"
        }
      ],
      "contract": {
        "states": [
          "probe-unset",
          "waiting-ready",
          "ready",
          "exited-before-ready",
          "ready-timeout"
        ],
        "transitions": [
          {
            "input": "start_with_connection on a manager with ready_probe == None",
            "state": "probe-unset",
            "effect": "no-op",
            "evidence": "anchor '    async fn failed_respawn_is_retried() {' — an existing test with no probe must stay green: wait_ready returns Ok immediately and spawn still means Running"
          },
          {
            "input": "launch() returned Ok, probe address accepts a TCP connection, and child.try_wait() is still None at or after STABILITY_WINDOW from spawn",
            "state": "ready",
            "effect": "set",
            "evidence": "requirement sentence: 'its local SOCKS (or mixed) inbound accepts a TCP connection and the process is still running at least one second after it was spawned' — only here does start_with_connection run `self.state.transition(ProcessState::Running, connection)?`"
          },
          {
            "input": "probe address accepts a TCP connection but child.try_wait() yields Some(status) before STABILITY_WINDOW elapsed",
            "state": "exited-before-ready",
            "effect": "forced",
            "evidence": "design decision: 'Port alone passes a sing-box that binds inbounds and then FATALs in post-start' — the accept does not win; the exit does"
          },
          {
            "input": "child.try_wait() yields Some(status) at any point during the wait",
            "state": "exited-before-ready",
            "effect": "forced",
            "evidence": "anchor '        let mut msg = match exit_code {' — wait_ready calls cleanup_after_exit(), then builds the identical `process exited with code {code}: {last_output_line}` text (`process killed by signal` when code is None), then write_exit_record(false, Some(&status)), then returns Err(ProcessError::ExitedBeforeReady(msg))"
          },
          {
            "input": "ready_timeout elapses with the child alive and no successful connect",
            "state": "ready-timeout",
            "effect": "forced",
            "evidence": "requirement sentence: 'When the backend does not become ready within 15 seconds, the system SHALL stop it and fail the start with an error naming the probed address and the timeout' — wait_ready calls graceful_stop() (which writes requested=true) and returns Err(ProcessError::ReadyTimeout { addr, timeout })"
          },
          {
            "input": "wait_ready returned Err inside start_with_connection (initial start)",
            "state": "exited-before-ready | ready-timeout",
            "effect": "forced",
            "evidence": "anchor '        match self.launch().await {' — the Err arm's shape is reused: transition to ProcessState::Error(e.to_string()) and return Err(e); record_crash() is NOT called and no respawn loop runs"
          },
          {
            "input": "wait_ready returned Err inside respawn()",
            "state": "exited-before-ready | ready-timeout",
            "effect": "forced",
            "evidence": "anchor '                Err(e) => {' in handle_unexpected_exit — the Err propagates out of respawn so the existing loop sets msg = `restart failed: {e}` and calls self.record_crash(), retrying while crash_times.len() < MAX_CRASHES"
          },
          {
            "input": "xray TUN start: device wait and xray_up both succeeded inside launch()",
            "state": "waiting-ready",
            "effect": "set",
            "evidence": "anchor '            let run = tun::xray_up(&rt).await;' — wait_ready is invoked by start_with_connection/respawn after launch() returns Ok, never inside launch() before the helper block"
          },
          {
            "input": "the caller drops the start_with_connection future while wait_ready is polling (Disconnect during Starting)",
            "state": "waiting-ready",
            "effect": "no-op",
            "evidence": "anchor '                Some(ConnectionCmd::Stop) = cmd_rx.recv() => {' in crates/ui/src/connection.rs — the manager writes nothing on cancel; mgr.shutdown() owns the teardown and the requested=true exit record"
          }
        ],
        "forbidden": [
          "record_crash() called for a readiness failure on the initial start — task 1.3 asserts crashes_in_window=0 in the exit record",
          "routing a self-exited-before-ready child through graceful_stop: with the child already reaped by wait_ready the record would read requested=true and mislabel a startup failure as a user stop",
          "two ' session ' records in backend.log for one candidate's initial start",
          "a `reason=` field in the exit record — the exit-reason vocabulary (start-failed / crash / requested) belongs to log-connection-decisions (#3) and lands after this change; #1 writes only requested=false plus the existing code=/signal=, crashes_in_window=, last_output= fields",
          "calling wait_ready from inside launch() — it would run before the xray device wait and xray_up",
          "ProcessError::ExitedBeforeReady or ProcessError::ReadyTimeout classified as host-level: a startup failure is per-candidate and must let failover continue",
          "reading last_output_line() before cleanup_after_exit() has drained the readers — the reason would be a stale snapshot",
          "polling the probe faster than READY_POLL_INTERVAL or logging a line per poll — a crash-looping backend would flood the very log this sprint is making legible",
          "A readiness TIMEOUT routes through graceful_stop and therefore writes `exit requested=true` with no reason field. That is correct today (the app did request the stop) and is NOT the exit-before-ready branch; log-connection-decisions gives it `reason=start-failed`. No test in this change asserts the timeout's exit record."
        ],
        "seeding": [
          "ready: manager_for(&dir, \"exec sleep 30\\n\") + mgr.ready_probe set through with_ready_probe to the addr of a `std::net::TcpListener::bind(\"127.0.0.1:0\")` the test keeps alive for the whole test body (do NOT drop it, unlike the bind-then-drop trick in crates/process/src/probe.rs), then mgr.start().await",
          "exited-before-ready: manager_for(&dir, \"echo FATAL >&2\\nexit 1\\n\") with a live listener and a ready probe, then mgr.start().await — never by killing the child from the test and never by assigning mgr.child",
          "ready-timeout: manager_for(&dir, \"exec sleep 30\\n\") with a probe address taken from a listener that is bound and then dropped (the crates/process/src/probe.rs `fake_sleeper_binary` pairing) plus `mgr.ready_timeout = Duration::from_millis(300);` — the field is private and set directly, exactly as `mgr.restart_delay = Duration::from_millis(50);` already is",
          "respawn readiness failure: a crashing_backend-shaped stub that serves once (exec sleep 30) then exits immediately on every later run, restart_delay 50 ms, a live listener, and repeated mgr.wait_and_handle_exit() until the state leaves Running",
          "probe-unset: any existing test untouched — no with_ready_probe call at all",
          "state is never seeded by calling self.state.transition directly from a test"
        ],
        "budgets": [
          "READY_TIMEOUT: Duration = Duration::from_secs(15) — the default ready_timeout",
          "STABILITY_WINDOW: Duration = Duration::from_secs(1) — measured from wait_ready entry (after launch(), so after the xray TUN device wait and xray-up), not from the first accept — the stricter variant, and it needs no extra field",
          "READY_POLL_INTERVAL: Duration = Duration::from_millis(100) — fixed interval, no backoff, so the 15 s deadline is at most 150 polls",
          "in-crate test override: ready_timeout 300 ms for the timeout case",
          "restart_delay 50 ms in respawn tests; MAX_CRASHES 3 within CRASH_WINDOW 60 s unchanged",
          "task 1.3 asserts no respawn within 3 s of the startup failure",
          "wait_for_lines polls for at most 2 s; LOG_DRAIN_TIMEOUT 500 ms unchanged",
          "one TCP connection per launch attempt, at most MAX_CRASHES+1 per session"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.4-t1: test `respawn_readiness_failure_exhausts_crash_budget` — stub that runs `exec sleep 30` on run 1 and exits 1 on every later run, live listener, ready probe, restart_delay 50 ms; loop wait_and_handle_exit() while Running; end state ProcessState::Error(msg) with msg containing \"3 crashes within 60s\"",
        "1.5-t1: assert the untouched path — run the existing suite; `failed_respawn_is_retried`, `respawn_skips_preflight`, `respawn_budget_exhaustion_errors`, `config_check_success_starts_backend`, `exit_record_marks_requested_stop`, `exit_record_marks_unrequested_crash` stay byte-identical and green",
        "1.4-c1: in respawn (anchor '    async fn respawn(&mut self) -> Result<(), ProcessError> {') insert `self.wait_ready().await?;` between `self.launch().await?;` and the Running transition"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-process && cargo clippy -p v2ray-rs-process --all-targets -- -D warnings",
      "coder": "rust-coder"
    },
    {
      "id": "r5",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "prev": "r4",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "seam": "ui-connection-ready-probe",
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "candidate manager builder chain",
          "anchor": "                .with_log_file(backend_log.clone()),",
          "change": "Append `.with_ready_probe(effective_settings.local_endpoint(effective_settings.socks_port))` to the chain — the only non-test production edit of task 2.1."
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests::singbox_settings / stub harness",
          "anchor": "    fn singbox_settings() -> AppSettings {",
          "change": "stub-backend tests bind an ephemeral listener and set settings.socks_port to its port so the readiness probe succeeds; add the helper next to this builder"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests::last_candidate_failure_reports_one_error",
          "anchor": "        assert!(msg.contains(\"203.0.113.1: 3 crashes\"), \"{msg}\");",
          "change": "expect the startup-failure reason (process exited with code …) instead of `3 crashes`"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests (new two-candidate readiness test)",
          "anchor": "    async fn live_connect_writes_backend_diagnostics() {",
          "change": "add test beside it: candidate 1 exits with FATAL right after start, candidate 2 serves -> exactly one ` session ` record for candidate 1, Running only reported for candidate 2"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "supervision loop exit paths (halt set)",
          "anchor": "            halt(state_forwarder).await;\n            halt(log_forwarder).await;\n            parked = Some(mgr);",
          "change": "The fall-through already halts both; the Stop arm and the `_ =>` arm inside the loop halt only state_forwarder. Replace all three with one helper that halts every spawned task, so detect-dead-proxy-link can register its health monitor with it."
        }
      ],
      "contract": {
        "states": [
          "candidate-started",
          "candidate-ready",
          "candidate-failed-before-ready",
          "stop-queued",
          "forwarders-live",
          "forwarders-halted"
        ],
        "transitions": [
          {
            "input": "a candidate is built",
            "state": "candidate-started",
            "effect": "set",
            "evidence": "anchor '                .with_log_file(backend_log.clone()),' — `.with_ready_probe(effective_settings.local_endpoint(effective_settings.socks_port))` is appended to the same chain, inside configure(), per candidate"
          },
          {
            "input": "resolve_effective_config overrode socks_port or listen_address for this node",
            "state": "candidate-started",
            "effect": "forced",
            "evidence": "anchor '            let (mut effective_rules, mut effective_settings) = resolve_effective_config(' — the probe address is read from effective_settings, never from the outer `settings`, because the generated config binds the effective values"
          },
          {
            "input": "start_with_connection returns Ok (the backend is ready)",
            "state": "candidate-ready",
            "effect": "set",
            "evidence": "anchor '                    report(ProcessState::Running, Some(meta.clone()));' — unchanged code, but Ok now means ready; the queued-Stop check above it still runs first"
          },
          {
            "input": "start_with_connection returns Err(ExitedBeforeReady) or Err(ReadyTimeout)",
            "state": "candidate-failed-before-ready",
            "effect": "forced",
            "evidence": "anchor '                    failures.push(CandidateFailure::new(' — is_host_level() is false for both variants, so the existing per-candidate Err branch parks the manager and moves to the next candidate with no crash-restart delay"
          },
          {
            "input": "Stop arrives inside the supervision loop while forwarders are live",
            "state": "forwarders-halted",
            "effect": "forced",
            "evidence": "anchors '                        mgr.shutdown().await;' / '                        halt(state_forwarder).await;' / '                        report(ProcessState::Stopped, None);' — both this arm and the `_ =>` arm must halt log_forwarder as well before their terminal report"
          },
          {
            "input": "the manager reaches a non-Running, non-Error state after wait_and_handle_exit",
            "state": "forwarders-halted",
            "effect": "forced",
            "evidence": "anchor '                            _ => {' — same halt set as the Stop arm"
          }
        ],
        "forbidden": [
          "reading the probe address from the outer `settings` instead of `effective_settings`",
          "a UI test that expects ProcessState::Running without a live listener bound to its settings.socks_port — ready_timeout is a private process-crate field the ui crate cannot shorten, so such a test would burn the full 15 s and push RECV_TIMEOUT (20 s) to the edge",
          "binding the listener and dropping it before the connection task runs",
          "reporting Running for a candidate whose stub exits right after start, even when a shared listener makes the port accept",
          "returning from an exit path with a forwarder still able to emit — assert_nothing_after_terminal exists precisely to catch it"
        ],
        "seeding": [
          "candidate-ready: in the test, `let listener = std::net::TcpListener::bind(\"127.0.0.1:0\").unwrap();` kept alive for the test body, `settings.socks_port = listener.local_addr().unwrap().port();` (listen_address stays the default \"127.0.0.1\"), then connect(&stub, settings, candidates) with a stub whose run branch is `exec sleep 30`",
          "candidate-failed-before-ready: same listener, a stub whose run branch prints FATAL to stderr and exits 1 for the matching candidate (the existing `grep -q <address> \"$3\"` shape selects which candidate fails)",
          "stop-queued: handle.stop() after the Running report, as failover_reports_no_stopped_and_stop_reports_one already does",
          "state is only ever observed through next_state / drain / assert_nothing_after_terminal; never by inspecting the manager"
        ],
        "budgets": [
          "RECV_TIMEOUT 20 s per message, unchanged",
          "+1 s (STABILITY_WINDOW) per candidate that reaches Running: failover_reports_no_stopped_and_stop_reports_one, log_lines_carry_connection_generation, live_connect_writes_backend_diagnostics, and the new two-candidate test",
          "a candidate that exits before ready costs one launch, no 2 s/4 s crash-restart delays",
          "exactly one ' session ' record per candidate attempt"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "2.1-t1: add a harness helper next to the anchor '    fn singbox_settings() -> AppSettings {' that binds a std::net::TcpListener on 127.0.0.1:0, returns it together with AppSettings whose socks_port is the bound port, and document that the listener must outlive the connect",
        "2.1-t2: rewrite the settings of every test that expects Running to use that helper: failover_reports_no_stopped_and_stop_reports_one, log_lines_carry_connection_generation, live_connect_writes_backend_diagnostics",
        "2.1-t3: at the anchor '        assert!(msg.contains(\"203.0.113.1: 3 crashes\"), \"{msg}\");' in last_candidate_failure_reports_one_error, expect the startup-failure reason instead — the candidate now fails once with `process exited with code 1` and never reaches the crash budget; keep the second assertion on \"203.0.113.3: config rejected\" as is",
        "2.2-t1: new test `two_candidates_first_exits_before_ready` beside the anchor '    async fn live_connect_writes_backend_diagnostics() {' — one shared listener; candidate 203.0.113.1's run branch prints FATAL and exits 1, candidate 203.0.113.2 runs `exec sleep 30`; assert Running is reported only with metadata naming 203.0.113.2, that no Running carried 203.0.113.1, and that backend.log holds exactly two \" session \" records (one per candidate attempt) with only one preceding the first candidate's exit record",
        "2.1-c1: append `.with_ready_probe(effective_settings.local_endpoint(effective_settings.socks_port))` to the builder chain at the anchor '                .with_log_file(backend_log.clone()),'",
        "2.1-c2: no import change is needed for the address helper (AppSettings is already imported); the process-crate `use v2ray_rs_process::{ProcessError, ProcessEvent, ProcessManager, ProcessState, TunRuntime};` line stays as is",
        "2.1-c3: replace the ad-hoc halts in the supervision loop with one helper that halts both state_forwarder and log_forwarder, and call it on every exit path that reports a terminal state — the Stop arm (anchor '                        halt(state_forwarder).await;'), the `_ =>` arm, and the existing fall-through after the loop; detect-dead-proxy-link registers its health task with the same helper",
        "2.1-c4: `make test-ui` green",
        "2.1-t4: add `stop_halts_every_forwarder` — connect a stub, `handle.stop()`, wait for Stopped, then assert no further backend log lines arrive on the buffer after the terminal report (the log forwarder is halted, not just the state forwarder)"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-ui && cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings",
      "coder": "rust-coder"
    },
    {
      "id": "r6",
      "taskIds": [
        "2.3"
      ],
      "prev": "r5",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "seam": "ui-app-last-success",
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "2.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "records_last_success (new pure fn)",
          "anchor": "fn last_success_settings(current: &AppSettings, meta: &ConnectionMetadata) -> AppSettings {",
          "change": "add pure records_last_success(&ProcessState, Option<&ConnectionMetadata>) -> bool next to it"
        },
        {
          "task": "2.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::ProcessStateConnection arm",
          "anchor": "                        let settings = last_success_settings(&self.settings, meta);",
          "change": "gate the persist on records_last_success(&state, self.connection_status.as_ref()) — `connection` is moved by the assignment above, so the `connection.as_ref()` form does not compile; Starting/Stopping relays still update connection_status only"
        },
        {
          "task": "2.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "tests",
          "anchor": "    fn direct_session_success_records_last_success() {",
          "change": "add unit test for records_last_success: Starting+meta false, Running+meta true, Running+none false"
        }
      ],
      "contract": {
        "states": [
          "displayed-only",
          "persisted"
        ],
        "transitions": [
          {
            "input": "ProcessStateConnection(gen, ProcessState::Running, Some(meta)) for the current generation",
            "state": "persisted",
            "effect": "set",
            "evidence": "anchor '                        let settings = last_success_settings(&self.settings, meta);' — the only surviving persist path; requirement sentence: 'The last-success record SHALL change only when a connection succeeds, meaning the connection is reported Running after its backend became ready.'"
          },
          {
            "input": "ProcessStateConnection(gen, ProcessState::Starting, Some(meta)) — the respawn relay",
            "state": "displayed-only",
            "effect": "no-op",
            "evidence": "requirement sentence: 'A connection that reaches only Starting, or whose backend exits or times out before it is ready, SHALL NOT change it.'; the relay source is `fn relays(state: &ProcessState) -> bool` in crates/ui/src/connection.rs"
          },
          {
            "input": "ProcessStateConnection(gen, ProcessState::Stopping, Some(meta))",
            "state": "displayed-only",
            "effect": "no-op",
            "evidence": "anchor '                if connection.is_some() {' — self.connection_status is still assigned; only the persist below it is gated"
          },
          {
            "input": "ProcessStateConnection(gen, ProcessState::Running, None)",
            "state": "displayed-only",
            "effect": "no-op",
            "evidence": "anchor '                } else if matches!(state, ProcessState::Stopped | ProcessState::Error(_)) {' — with no metadata there is nothing to record; records_last_success returns false"
          },
          {
            "input": "ProcessStateConnection for a superseded generation",
            "state": "displayed-only",
            "effect": "no-op",
            "evidence": "anchor '                if !is_current_generation(generation, self.connection_generation) {' — the existing guard returns before any of this; unchanged"
          },
          {
            "input": "a settings flush from the preferences dialog",
            "state": "displayed-only",
            "effect": "no-op",
            "evidence": "anchor '                let settings = keep_last_success(settings, &self.settings);' — unchanged; requirement sentence: 'Saving settings from the preferences dialog SHALL NOT modify it.'"
          }
        ],
        "forbidden": [
          "clearing an existing last_success when a start fails before ready — the record keeps naming the previously successful node (spec scenario 'Backend dies before ready')",
          "gating self.connection_status on records_last_success: the status bar must still follow Starting and Stopping relays",
          "persisting settings on any state other than Running"
        ],
        "seeding": [
          "the pure fn is called directly with constructed ProcessState values and an Option<&ConnectionMetadata> built like the one in direct_session_success_records_last_success; no App instance, no GTK, no persistence — app.rs tests are otherwise pure"
        ],
        "budgets": [
          "3 assertion rows: Starting+meta false, Running+meta true, Running+None false",
          "at most one persist_settings call per Running report"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "2.3-t1: test `records_last_success_only_on_running` beside the anchor '    fn direct_session_success_records_last_success() {' asserting the three rows, plus Stopping+meta false",
        "2.3-c1: add `fn records_last_success(state: &ProcessState, connection: Option<&ConnectionMetadata>) -> bool` next to the anchor 'fn last_success_settings(current: &AppSettings, meta: &ConnectionMetadata) -> AppSettings {' — true only for `(ProcessState::Running, Some(_))`",
        "2.3-c2: gate the persist at the anchor '                        let settings = last_success_settings(&self.settings, meta);' on records_last_success(&state, self.connection_status.as_ref()) — computing it from `connection` after `self.connection_status = connection;` borrows a moved value and does not compile; leave the `self.connection_status = connection;` assignment above it untouched",
        "2.3-c3: `make test-ui` green"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-ui && cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings",
      "coder": "rust-coder"
    },
    {
      "id": "r7",
      "taskIds": [
        "3.1",
        "3.2"
      ],
      "prev": "r6",
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
          "automated-floor",
          "manual-live"
        ],
        "transitions": [
          {
            "input": "the full workspace suite",
            "state": "automated-floor",
            "effect": "set",
            "evidence": "Makefile anchor 'TEST := timeout $(TEST_TIMEOUT) $(CARGO) test' with TEST_ARGS '-- --test-threads=$(TEST_THREADS)'"
          },
          {
            "input": "sing-box TUN on an IPv6-less host",
            "state": "manual-live",
            "effect": "no-op",
            "evidence": "tasks.md 3.2 — status never shows Connected, one ' session ' per candidate with no 2 s/4 s respawns, settings.toml last_success unchanged"
          },
          {
            "input": "xray TUN on a working node",
            "state": "manual-live",
            "effect": "set",
            "evidence": "tasks.md 3.2 — Connected only after routes are up, last_success updated"
          }
        ],
        "forbidden": [
          "a bare `cargo test` without the timeout and --test-threads cap",
          "treating 3.2 as done from the automated suite"
        ],
        "seeding": [
          "automated-floor: `make test` from the repo root (TEST_TIMEOUT 5m, TEST_THREADS 4 are Makefile defaults; raise with `make test TEST_TIMEOUT=10m` for the workspace run)",
          "manual-live: the operator runs the app against an installed backend; report the backend.log excerpt and the settings.toml last_success value"
        ],
        "budgets": [
          "workspace run bounded at 10m, --test-threads=4",
          "per-crate runs bounded at 5m"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "3.1-t1: `make test TEST_TIMEOUT=10m` green (equivalently `timeout 10m cargo test --workspace --all-targets -- --test-threads=4`)",
        "3.1-t2: `make lint` green (cargo fmt --check plus cargo clippy --workspace --all-targets --all-features -- -D warnings)",
        "3.2-m1: MANUAL — sing-box TUN on an IPv6-less host: status never reaches Connected, backend.log has one ' session ' per candidate with no 2 s/4 s respawn pairs, settings.toml last_success unchanged",
        "3.2-m2: MANUAL — xray TUN on a working node: Connected only after the device wait and xray-up succeed, and settings.toml last_success updated"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test TEST_TIMEOUT=10m && make lint",
      "coder": "rust-coder"
    }
  ],
  "seams": [
    {
      "id": "core-local-endpoint",
      "tasks": [
        "1.1"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no red-stage agent, so no sealed test pass precedes the coder. NO-TESTER-WAIVER: Rust stack has no test-writer agent; the coder writes the tests first inside its own seam. Pure helper AppSettings::local_endpoint(port) -> SocketAddr in crates/core/src/models/settings.rs, beside AppSettings::validate_listen_address, turning the listen_address IP literal plus a port into a probeable SocketAddr: unspecified v4 (0.0.0.0) -> 127.0.0.1, unspecified v6 (::) -> ::1, anything else kept as written, an unparsable literal falling back to 127.0.0.1 so a hand-edited settings.toml cannot make the readiness probe panic. detect-dead-proxy-link calls the same helper with http_port, so it takes &self and a port rather than being bound to socks_port. Tests may be added only to the existing `mod tests` at the bottom of crates/core/src/models/settings.rs (the file already holds test_validate_listen_address with exactly this address table).",
      "contract": {
        "states": [
          "loopback-v4",
          "unspecified-v4",
          "unspecified-v6",
          "explicit-v4",
          "unparsable"
        ],
        "transitions": [
          {
            "input": "listen_address \"127.0.0.1\", port 1080",
            "state": "loopback-v4",
            "effect": "no-op",
            "evidence": "anchor 'pub fn validate_listen_address(addr: &str) -> Result<(), ValidationError> {' — the literal is already a probeable address; local_endpoint returns 127.0.0.1:1080"
          },
          {
            "input": "listen_address \"0.0.0.0\", port 1080",
            "state": "unspecified-v4",
            "effect": "forced",
            "evidence": "requirement sentence: 'An unspecified listen address SHALL be probed on the loopback address of the same family.' — forced to 127.0.0.1:1080"
          },
          {
            "input": "listen_address \"::\", port 1080",
            "state": "unspecified-v6",
            "effect": "forced",
            "evidence": "requirement sentence: 'An unspecified listen address SHALL be probed on the loopback address of the same family.' — forced to [::1]:1080"
          },
          {
            "input": "listen_address \"192.168.1.10\", port 1080",
            "state": "explicit-v4",
            "effect": "no-op",
            "evidence": "anchor '        let valid = [\"127.0.0.1\", \"0.0.0.0\", \"::\", \"::1\", \"192.168.1.10\"];' — an explicit bind address is probed as written"
          },
          {
            "input": "listen_address that IpAddr::from_str rejects (\"\", \"localhost\")",
            "state": "unparsable",
            "effect": "forced",
            "evidence": "anchor '        IpAddr::from_str(addr)' — validate_listen_address rejects these at edit time, but a loaded settings.toml can still carry one; local_endpoint forces 127.0.0.1:<port> instead of panicking or unwrapping"
          }
        ],
        "forbidden": [
          "local_endpoint returning an unspecified address (0.0.0.0 or ::) — nothing can TCP-connect to it",
          "local_endpoint panicking or returning Result — every caller sits on a start path and has no recovery to offer",
          "a second address formatter: crates/ui/src/connection.rs keeps `fn listen_endpoint(addr: &str, port: u16) -> String` for the human-facing v2ray warning text (bracketed display form), and it is NOT replaced by local_endpoint in this change"
        ],
        "seeding": [
          "every state is reached by constructing AppSettings::default() and assigning settings.listen_address plus the port argument; no file I/O, no persistence"
        ],
        "budgets": [
          "pure function, no await, no syscall — 0 ms of wall clock",
          "5 assertion cases: 127.0.0.1, 0.0.0.0, ::, 192.168.1.10, and one unparsable literal"
        ]
      },
      "codeTasks": [
        "1.1-t1: in crates/core/src/models/settings.rs `mod tests`, next to `fn test_validate_listen_address()`, add `fn local_endpoint_maps_unspecified_to_loopback()` asserting the five rows of the transition table as SocketAddr values (127.0.0.1:1080, 127.0.0.1:1080, [::1]:1080, 192.168.1.10:1080, 127.0.0.1:1080)",
        "1.1-t2: add `pub fn local_endpoint(&self, port: u16) -> SocketAddr` to the `impl AppSettings` block that holds validate_listen_address; parse self.listen_address with IpAddr::from_str (already imported via `use std::str::FromStr;` and `use std::net::IpAddr;`), map IpAddr::is_unspecified to Ipv4Addr::LOCALHOST / Ipv6Addr::LOCALHOST of the same family, and fall back to Ipv4Addr::LOCALHOST on a parse error; extend the `use std::net::` line to bring in SocketAddr, Ipv4Addr, Ipv6Addr",
        "1.1-t3: `make test-core` green"
      ]
    },
    {
      "id": "process-readiness",
      "tasks": [
        "1.2",
        "1.3",
        "1.4",
        "1.5"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no red-stage agent. NO-TESTER-WAIVER: Rust stack has no test-writer agent; the tests are the first codeTasks of this seam. The readiness gate itself, entirely inside crates/process/src/manager.rs: two new ProcessError variants, an opt-in `with_ready_probe` builder, a private `wait_ready` between launch() and the Running transition in both start_with_connection and respawn, and the pre-ready exit record. No change to crates/process/src/lib.rs is needed — the sprint put the address helper on AppSettings in core (task 1.1), so crates/ui never names a process-crate free function and nothing new has to be re-exported. Tests may be added only to the existing `mod tests` in crates/process/src/manager.rs, using write_script / manager_for / backend_log / crashing_backend / read_lines / wait_for_lines / drain_states / stub_helper / xray_on_lo as they stand.",
      "contract": {
        "states": [
          "probe-unset",
          "waiting-ready",
          "ready",
          "exited-before-ready",
          "ready-timeout"
        ],
        "transitions": [
          {
            "input": "start_with_connection on a manager with ready_probe == None",
            "state": "probe-unset",
            "effect": "no-op",
            "evidence": "anchor '    async fn failed_respawn_is_retried() {' — an existing test with no probe must stay green: wait_ready returns Ok immediately and spawn still means Running"
          },
          {
            "input": "launch() returned Ok, probe address accepts a TCP connection, and child.try_wait() is still None at or after STABILITY_WINDOW from spawn",
            "state": "ready",
            "effect": "set",
            "evidence": "requirement sentence: 'its local SOCKS (or mixed) inbound accepts a TCP connection and the process is still running at least one second after it was spawned' — only here does start_with_connection run `self.state.transition(ProcessState::Running, connection)?`"
          },
          {
            "input": "probe address accepts a TCP connection but child.try_wait() yields Some(status) before STABILITY_WINDOW elapsed",
            "state": "exited-before-ready",
            "effect": "forced",
            "evidence": "design decision: 'Port alone passes a sing-box that binds inbounds and then FATALs in post-start' — the accept does not win; the exit does"
          },
          {
            "input": "child.try_wait() yields Some(status) at any point during the wait",
            "state": "exited-before-ready",
            "effect": "forced",
            "evidence": "anchor '        let mut msg = match exit_code {' — wait_ready calls cleanup_after_exit(), then builds the identical `process exited with code {code}: {last_output_line}` text (`process killed by signal` when code is None), then write_exit_record(false, Some(&status)), then returns Err(ProcessError::ExitedBeforeReady(msg))"
          },
          {
            "input": "ready_timeout elapses with the child alive and no successful connect",
            "state": "ready-timeout",
            "effect": "forced",
            "evidence": "requirement sentence: 'When the backend does not become ready within 15 seconds, the system SHALL stop it and fail the start with an error naming the probed address and the timeout' — wait_ready calls graceful_stop() (which writes requested=true) and returns Err(ProcessError::ReadyTimeout { addr, timeout })"
          },
          {
            "input": "wait_ready returned Err inside start_with_connection (initial start)",
            "state": "exited-before-ready | ready-timeout",
            "effect": "forced",
            "evidence": "anchor '        match self.launch().await {' — the Err arm's shape is reused: transition to ProcessState::Error(e.to_string()) and return Err(e); record_crash() is NOT called and no respawn loop runs"
          },
          {
            "input": "wait_ready returned Err inside respawn()",
            "state": "exited-before-ready | ready-timeout",
            "effect": "forced",
            "evidence": "anchor '                Err(e) => {' in handle_unexpected_exit — the Err propagates out of respawn so the existing loop sets msg = `restart failed: {e}` and calls self.record_crash(), retrying while crash_times.len() < MAX_CRASHES"
          },
          {
            "input": "xray TUN start: device wait and xray_up both succeeded inside launch()",
            "state": "waiting-ready",
            "effect": "set",
            "evidence": "anchor '            let run = tun::xray_up(&rt).await;' — wait_ready is invoked by start_with_connection/respawn after launch() returns Ok, never inside launch() before the helper block"
          },
          {
            "input": "the caller drops the start_with_connection future while wait_ready is polling (Disconnect during Starting)",
            "state": "waiting-ready",
            "effect": "no-op",
            "evidence": "anchor '                Some(ConnectionCmd::Stop) = cmd_rx.recv() => {' in crates/ui/src/connection.rs — the manager writes nothing on cancel; mgr.shutdown() owns the teardown and the requested=true exit record"
          }
        ],
        "forbidden": [
          "record_crash() called for a readiness failure on the initial start — task 1.3 asserts crashes_in_window=0 in the exit record",
          "routing a self-exited-before-ready child through graceful_stop: with the child already reaped by wait_ready the record would read requested=true and mislabel a startup failure as a user stop",
          "two ' session ' records in backend.log for one candidate's initial start",
          "a `reason=` field in the exit record — the exit-reason vocabulary (start-failed / crash / requested) belongs to log-connection-decisions (#3) and lands after this change; #1 writes only requested=false plus the existing code=/signal=, crashes_in_window=, last_output= fields",
          "calling wait_ready from inside launch() — it would run before the xray device wait and xray_up",
          "ProcessError::ExitedBeforeReady or ProcessError::ReadyTimeout classified as host-level: a startup failure is per-candidate and must let failover continue",
          "reading last_output_line() before cleanup_after_exit() has drained the readers — the reason would be a stale snapshot",
          "polling the probe faster than READY_POLL_INTERVAL or logging a line per poll — a crash-looping backend would flood the very log this sprint is making legible",
          "A readiness TIMEOUT routes through graceful_stop and therefore writes `exit requested=true` with no reason field. That is correct today (the app did request the stop) and is NOT the exit-before-ready branch; log-connection-decisions gives it `reason=start-failed`. No test in this change asserts the timeout's exit record."
        ],
        "seeding": [
          "ready: manager_for(&dir, \"exec sleep 30\\n\") + mgr.ready_probe set through with_ready_probe to the addr of a `std::net::TcpListener::bind(\"127.0.0.1:0\")` the test keeps alive for the whole test body (do NOT drop it, unlike the bind-then-drop trick in crates/process/src/probe.rs), then mgr.start().await",
          "exited-before-ready: manager_for(&dir, \"echo FATAL >&2\\nexit 1\\n\") with a live listener and a ready probe, then mgr.start().await — never by killing the child from the test and never by assigning mgr.child",
          "ready-timeout: manager_for(&dir, \"exec sleep 30\\n\") with a probe address taken from a listener that is bound and then dropped (the crates/process/src/probe.rs `fake_sleeper_binary` pairing) plus `mgr.ready_timeout = Duration::from_millis(300);` — the field is private and set directly, exactly as `mgr.restart_delay = Duration::from_millis(50);` already is",
          "respawn readiness failure: a crashing_backend-shaped stub that serves once (exec sleep 30) then exits immediately on every later run, restart_delay 50 ms, a live listener, and repeated mgr.wait_and_handle_exit() until the state leaves Running",
          "probe-unset: any existing test untouched — no with_ready_probe call at all",
          "state is never seeded by calling self.state.transition directly from a test"
        ],
        "budgets": [
          "READY_TIMEOUT: Duration = Duration::from_secs(15) — the default ready_timeout",
          "STABILITY_WINDOW: Duration = Duration::from_secs(1) — measured from wait_ready entry (after launch(), so after the xray TUN device wait and xray-up), not from the first accept — the stricter variant, and it needs no extra field",
          "READY_POLL_INTERVAL: Duration = Duration::from_millis(100) — fixed interval, no backoff, so the 15 s deadline is at most 150 polls",
          "in-crate test override: ready_timeout 300 ms for the timeout case",
          "restart_delay 50 ms in respawn tests; MAX_CRASHES 3 within CRASH_WINDOW 60 s unchanged",
          "task 1.3 asserts no respawn within 3 s of the startup failure",
          "wait_for_lines polls for at most 2 s; LOG_DRAIN_TIMEOUT 500 ms unchanged",
          "one TCP connection per launch attempt, at most MAX_CRASHES+1 per session"
        ]
      },
      "codeTasks": [
        "1.2-t1: test `ready_probe_accepts_live_backend` — listener alive, `exec sleep 30` stub, with_ready_probe(listener addr); start() -> Ok, mgr.state() == ProcessState::Running; assert the start took at least STABILITY_WINDOW",
        "1.2-t2: test `exit_before_ready_carries_last_output` — stub `echo FATAL >&2` then `exit 1`, live listener, with_ready_probe; start() -> Err(ProcessError::ExitedBeforeReady(msg)) with msg containing \"code 1\" and \"FATAL\"",
        "1.2-t3: test `ready_timeout_stops_the_backend` — `exec sleep 30` stub, probe address from a bound-then-dropped listener, mgr.ready_timeout = 300 ms; start() -> Err(ProcessError::ReadyTimeout { .. }) whose Display names the address and `300ms`; mgr.child.is_none() afterwards (the child was reaped by graceful_stop)",
        "1.2-c1: add `const READY_TIMEOUT: Duration = Duration::from_secs(15);`, `const STABILITY_WINDOW: Duration = Duration::from_secs(1);` and `const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);` beside the anchor 'const CRASH_RESTART_DELAY: Duration = Duration::from_secs(2);'",
        "1.2-c2: add to the ProcessError enum, after the anchor '    #[error(\"config rejected by backend: {0}\")]' variant: `#[error(\"{0}\")] ExitedBeforeReady(String)` and `#[error(\"backend did not accept connections on {addr} within {timeout:?}\")] ReadyTimeout { addr: SocketAddr, timeout: Duration }`; import std::net::SocketAddr",
        "1.2-c3: extend the `is_host_level_classifies_every_variant` case table with both new variants mapped to false (the test enumerates every variant and is the project's standing guard)",
        "1.2-c4: add fields `ready_probe: Option<SocketAddr>` and `ready_timeout: Duration` beside the anchor '    restart_delay: Duration,'; initialise them to None and READY_TIMEOUT beside the anchor '            restart_delay: CRASH_RESTART_DELAY,'",
        "1.2-c5: add `pub fn with_ready_probe(mut self, addr: SocketAddr) -> Self` next to the anchor '    pub fn with_tun(mut self, tun: Option<TunRuntime>) -> Self {', following the consuming-builder pattern",
        "1.2-c6: extract the exit-reason text at the anchor '        let mut msg = match exit_code {' into a private helper (status + last_output_line -> String) and call it from both handle_unexpected_exit and wait_ready, so the ExitedBeforeReady text is identical to a crash reason and failure_key normalisation keeps working",
        "1.2-c7: add `async fn wait_ready(&mut self) -> Result<(), ProcessError>` after the anchor '    // A launch failure leaves routing state in place: during a respawn the' — Ok(()) at once when ready_probe is None; otherwise record `probe_start: Instant` at wait_ready entry, loop every READY_POLL_INTERVAL: child.try_wait() Some(status) -> cleanup_after_exit().await, write_exit_record(false, Some(&status)), Err(ExitedBeforeReady(reason)); tokio::time::timeout(READY_POLL_INTERVAL, TcpStream::connect(addr)) Ok and probe_start.elapsed() >= STABILITY_WINDOW -> Ok(()) — an unbounded connect to a non-loopback address behind a drop rule outlives ready_timeout and the 15 s budget stops being enforced; deadline passed -> graceful_stop().await, Err(ReadyTimeout { addr, timeout: self.ready_timeout })",
        "1.3-t1: test `startup_failure_writes_one_session_and_no_crash` — FATAL-then-exit stub with .with_log_file(Some(backend_log(dir.path()))) and a ready probe; start() -> Err(ExitedBeforeReady); backend.log holds exactly one \" session \" record; the exit record contains \"exit requested=false\", \"code=1\", \"crashes_in_window=0\" and \"last_output=FATAL\"; after sleeping 3 s the log still holds exactly one session record (no respawn)",
        "1.3-c1: in start_with_connection at the anchor '        match self.launch().await {', change the Ok(()) arm to `self.wait_ready().await` first and transition to Running only on its Ok; on Err reuse the existing Err arm shape — transition to ProcessState::Error(e.to_string()) and return Err(e), with no record_crash and no graceful_stop of an already-reaped child",
        "1.4-t1: test `respawn_readiness_failure_exhausts_crash_budget` — stub that runs `exec sleep 30` on run 1 and exits 1 on every later run, live listener, ready probe, restart_delay 50 ms; loop wait_and_handle_exit() while Running; end state ProcessState::Error(msg) with msg containing \"3 crashes within 60s\"",
        "1.5-t1: assert the untouched path — run the existing suite; `failed_respawn_is_retried`, `respawn_skips_preflight`, `respawn_budget_exhaustion_errors`, `config_check_success_starts_backend`, `exit_record_marks_requested_stop`, `exit_record_marks_unrequested_crash` stay byte-identical and green",
        "1.4-c1: in respawn (anchor '    async fn respawn(&mut self) -> Result<(), ProcessError> {') insert `self.wait_ready().await?;` between `self.launch().await?;` and the Running transition"
      ]
    },
    {
      "id": "ui-connection-ready-probe",
      "tasks": [
        "2.1",
        "2.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no red-stage agent. NO-TESTER-WAIVER: Rust stack has no test-writer agent; the tests lead this seam's codeTasks. crates/ui/src/connection.rs only: append `.with_ready_probe(effective_settings.local_endpoint(effective_settings.socks_port))` to the per-candidate builder chain, and repair the stub-test harness so every test that expects Running actually makes the probe succeed. This is the first of the three sprint changes to enter the candidate loop, so it also owns the loop's fixed 6-step contract: builder chain, start, honour a queued Stop, report(Running), spawn forwarders, and halt every spawned task on every exit path before the terminal report — the `Some(ConnectionCmd::Stop)` arm and the `_ =>` arm today halt only state_forwarder and leak log_forwarder. Tests may be added or changed only inside the existing `mod tests` of crates/ui/src/connection.rs.",
      "contract": {
        "states": [
          "candidate-started",
          "candidate-ready",
          "candidate-failed-before-ready",
          "stop-queued",
          "forwarders-live",
          "forwarders-halted"
        ],
        "transitions": [
          {
            "input": "a candidate is built",
            "state": "candidate-started",
            "effect": "set",
            "evidence": "anchor '                .with_log_file(backend_log.clone()),' — `.with_ready_probe(effective_settings.local_endpoint(effective_settings.socks_port))` is appended to the same chain, inside configure(), per candidate"
          },
          {
            "input": "resolve_effective_config overrode socks_port or listen_address for this node",
            "state": "candidate-started",
            "effect": "forced",
            "evidence": "anchor '            let (mut effective_rules, mut effective_settings) = resolve_effective_config(' — the probe address is read from effective_settings, never from the outer `settings`, because the generated config binds the effective values"
          },
          {
            "input": "start_with_connection returns Ok (the backend is ready)",
            "state": "candidate-ready",
            "effect": "set",
            "evidence": "anchor '                    report(ProcessState::Running, Some(meta.clone()));' — unchanged code, but Ok now means ready; the queued-Stop check above it still runs first"
          },
          {
            "input": "start_with_connection returns Err(ExitedBeforeReady) or Err(ReadyTimeout)",
            "state": "candidate-failed-before-ready",
            "effect": "forced",
            "evidence": "anchor '                    failures.push(CandidateFailure::new(' — is_host_level() is false for both variants, so the existing per-candidate Err branch parks the manager and moves to the next candidate with no crash-restart delay"
          },
          {
            "input": "Stop arrives inside the supervision loop while forwarders are live",
            "state": "forwarders-halted",
            "effect": "forced",
            "evidence": "anchors '                        mgr.shutdown().await;' / '                        halt(state_forwarder).await;' / '                        report(ProcessState::Stopped, None);' — both this arm and the `_ =>` arm must halt log_forwarder as well before their terminal report"
          },
          {
            "input": "the manager reaches a non-Running, non-Error state after wait_and_handle_exit",
            "state": "forwarders-halted",
            "effect": "forced",
            "evidence": "anchor '                            _ => {' — same halt set as the Stop arm"
          }
        ],
        "forbidden": [
          "reading the probe address from the outer `settings` instead of `effective_settings`",
          "a UI test that expects ProcessState::Running without a live listener bound to its settings.socks_port — ready_timeout is a private process-crate field the ui crate cannot shorten, so such a test would burn the full 15 s and push RECV_TIMEOUT (20 s) to the edge",
          "binding the listener and dropping it before the connection task runs",
          "reporting Running for a candidate whose stub exits right after start, even when a shared listener makes the port accept",
          "returning from an exit path with a forwarder still able to emit — assert_nothing_after_terminal exists precisely to catch it"
        ],
        "seeding": [
          "candidate-ready: in the test, `let listener = std::net::TcpListener::bind(\"127.0.0.1:0\").unwrap();` kept alive for the test body, `settings.socks_port = listener.local_addr().unwrap().port();` (listen_address stays the default \"127.0.0.1\"), then connect(&stub, settings, candidates) with a stub whose run branch is `exec sleep 30`",
          "candidate-failed-before-ready: same listener, a stub whose run branch prints FATAL to stderr and exits 1 for the matching candidate (the existing `grep -q <address> \"$3\"` shape selects which candidate fails)",
          "stop-queued: handle.stop() after the Running report, as failover_reports_no_stopped_and_stop_reports_one already does",
          "state is only ever observed through next_state / drain / assert_nothing_after_terminal; never by inspecting the manager"
        ],
        "budgets": [
          "RECV_TIMEOUT 20 s per message, unchanged",
          "+1 s (STABILITY_WINDOW) per candidate that reaches Running: failover_reports_no_stopped_and_stop_reports_one, log_lines_carry_connection_generation, live_connect_writes_backend_diagnostics, and the new two-candidate test",
          "a candidate that exits before ready costs one launch, no 2 s/4 s crash-restart delays",
          "exactly one ' session ' record per candidate attempt"
        ]
      },
      "codeTasks": [
        "2.1-t1: add a harness helper next to the anchor '    fn singbox_settings() -> AppSettings {' that binds a std::net::TcpListener on 127.0.0.1:0, returns it together with AppSettings whose socks_port is the bound port, and document that the listener must outlive the connect",
        "2.1-t2: rewrite the settings of every test that expects Running to use that helper: failover_reports_no_stopped_and_stop_reports_one, log_lines_carry_connection_generation, live_connect_writes_backend_diagnostics",
        "2.1-t3: at the anchor '        assert!(msg.contains(\"203.0.113.1: 3 crashes\"), \"{msg}\");' in last_candidate_failure_reports_one_error, expect the startup-failure reason instead — the candidate now fails once with `process exited with code 1` and never reaches the crash budget; keep the second assertion on \"203.0.113.3: config rejected\" as is",
        "2.2-t1: new test `two_candidates_first_exits_before_ready` beside the anchor '    async fn live_connect_writes_backend_diagnostics() {' — one shared listener; candidate 203.0.113.1's run branch prints FATAL and exits 1, candidate 203.0.113.2 runs `exec sleep 30`; assert Running is reported only with metadata naming 203.0.113.2, that no Running carried 203.0.113.1, and that backend.log holds exactly two \" session \" records (one per candidate attempt) with only one preceding the first candidate's exit record",
        "2.1-c1: append `.with_ready_probe(effective_settings.local_endpoint(effective_settings.socks_port))` to the builder chain at the anchor '                .with_log_file(backend_log.clone()),'",
        "2.1-c2: no import change is needed for the address helper (AppSettings is already imported); the process-crate `use v2ray_rs_process::{ProcessError, ProcessEvent, ProcessManager, ProcessState, TunRuntime};` line stays as is",
        "2.1-c3: replace the ad-hoc halts in the supervision loop with one helper that halts both state_forwarder and log_forwarder, and call it on every exit path that reports a terminal state — the Stop arm (anchor '                        halt(state_forwarder).await;'), the `_ =>` arm, and the existing fall-through after the loop; detect-dead-proxy-link registers its health task with the same helper",
        "2.1-c4: `make test-ui` green",
        "2.1-t4: add `stop_halts_every_forwarder` — connect a stub, `handle.stop()`, wait for Stopped, then assert no further backend log lines arrive on the buffer after the terminal report (the log forwarder is halted, not just the state forwarder)"
      ]
    },
    {
      "id": "ui-app-last-success",
      "tasks": [
        "2.3"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no red-stage agent. NO-TESTER-WAIVER: Rust stack has no test-writer agent; the unit test is this seam's first codeTask. crates/ui/src/app.rs only: a pure `records_last_success(&ProcessState, Option<&ConnectionMetadata>) -> bool` joining the free-fn group at the bottom of the module (relays, repeats_previous, keep_last_success, last_success_settings), and a gate on it in the AppMsg::ProcessStateConnection arm so a Starting or Stopping relay that carries metadata still updates connection_status but no longer rewrites and persists last_success. Tests belong in the existing `mod tests` of crates/ui/src/app.rs, beside direct_session_success_records_last_success.",
      "contract": {
        "states": [
          "displayed-only",
          "persisted"
        ],
        "transitions": [
          {
            "input": "ProcessStateConnection(gen, ProcessState::Running, Some(meta)) for the current generation",
            "state": "persisted",
            "effect": "set",
            "evidence": "anchor '                        let settings = last_success_settings(&self.settings, meta);' — the only surviving persist path; requirement sentence: 'The last-success record SHALL change only when a connection succeeds, meaning the connection is reported Running after its backend became ready.'"
          },
          {
            "input": "ProcessStateConnection(gen, ProcessState::Starting, Some(meta)) — the respawn relay",
            "state": "displayed-only",
            "effect": "no-op",
            "evidence": "requirement sentence: 'A connection that reaches only Starting, or whose backend exits or times out before it is ready, SHALL NOT change it.'; the relay source is `fn relays(state: &ProcessState) -> bool` in crates/ui/src/connection.rs"
          },
          {
            "input": "ProcessStateConnection(gen, ProcessState::Stopping, Some(meta))",
            "state": "displayed-only",
            "effect": "no-op",
            "evidence": "anchor '                if connection.is_some() {' — self.connection_status is still assigned; only the persist below it is gated"
          },
          {
            "input": "ProcessStateConnection(gen, ProcessState::Running, None)",
            "state": "displayed-only",
            "effect": "no-op",
            "evidence": "anchor '                } else if matches!(state, ProcessState::Stopped | ProcessState::Error(_)) {' — with no metadata there is nothing to record; records_last_success returns false"
          },
          {
            "input": "ProcessStateConnection for a superseded generation",
            "state": "displayed-only",
            "effect": "no-op",
            "evidence": "anchor '                if !is_current_generation(generation, self.connection_generation) {' — the existing guard returns before any of this; unchanged"
          },
          {
            "input": "a settings flush from the preferences dialog",
            "state": "displayed-only",
            "effect": "no-op",
            "evidence": "anchor '                let settings = keep_last_success(settings, &self.settings);' — unchanged; requirement sentence: 'Saving settings from the preferences dialog SHALL NOT modify it.'"
          }
        ],
        "forbidden": [
          "clearing an existing last_success when a start fails before ready — the record keeps naming the previously successful node (spec scenario 'Backend dies before ready')",
          "gating self.connection_status on records_last_success: the status bar must still follow Starting and Stopping relays",
          "persisting settings on any state other than Running"
        ],
        "seeding": [
          "the pure fn is called directly with constructed ProcessState values and an Option<&ConnectionMetadata> built like the one in direct_session_success_records_last_success; no App instance, no GTK, no persistence — app.rs tests are otherwise pure"
        ],
        "budgets": [
          "3 assertion rows: Starting+meta false, Running+meta true, Running+None false",
          "at most one persist_settings call per Running report"
        ]
      },
      "codeTasks": [
        "2.3-t1: test `records_last_success_only_on_running` beside the anchor '    fn direct_session_success_records_last_success() {' asserting the three rows, plus Stopping+meta false",
        "2.3-c1: add `fn records_last_success(state: &ProcessState, connection: Option<&ConnectionMetadata>) -> bool` next to the anchor 'fn last_success_settings(current: &AppSettings, meta: &ConnectionMetadata) -> AppSettings {' — true only for `(ProcessState::Running, Some(_))`",
        "2.3-c2: gate the persist at the anchor '                        let settings = last_success_settings(&self.settings, meta);' on records_last_success(&state, self.connection_status.as_ref()) — computing it from `connection` after `self.connection_status = connection;` borrows a moved value and does not compile; leave the `self.connection_status = connection;` assignment above it untouched",
        "2.3-c3: `make test-ui` green"
      ]
    },
    {
      "id": "verification",
      "tasks": [
        "3.1",
        "3.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no red-stage agent. NO-TESTER-WAIVER: Rust stack has no test-writer agent. Whole-change floor plus the one manual step. 3.2 needs a real sing-box/xray, a real node and TUN privileges, so it is a manual verification step, not an automated task and not a blocker.",
      "contract": {
        "states": [
          "automated-floor",
          "manual-live"
        ],
        "transitions": [
          {
            "input": "the full workspace suite",
            "state": "automated-floor",
            "effect": "set",
            "evidence": "Makefile anchor 'TEST := timeout $(TEST_TIMEOUT) $(CARGO) test' with TEST_ARGS '-- --test-threads=$(TEST_THREADS)'"
          },
          {
            "input": "sing-box TUN on an IPv6-less host",
            "state": "manual-live",
            "effect": "no-op",
            "evidence": "tasks.md 3.2 — status never shows Connected, one ' session ' per candidate with no 2 s/4 s respawns, settings.toml last_success unchanged"
          },
          {
            "input": "xray TUN on a working node",
            "state": "manual-live",
            "effect": "set",
            "evidence": "tasks.md 3.2 — Connected only after routes are up, last_success updated"
          }
        ],
        "forbidden": [
          "a bare `cargo test` without the timeout and --test-threads cap",
          "treating 3.2 as done from the automated suite"
        ],
        "seeding": [
          "automated-floor: `make test` from the repo root (TEST_TIMEOUT 5m, TEST_THREADS 4 are Makefile defaults; raise with `make test TEST_TIMEOUT=10m` for the workspace run)",
          "manual-live: the operator runs the app against an installed backend; report the backend.log excerpt and the settings.toml last_success value"
        ],
        "budgets": [
          "workspace run bounded at 10m, --test-threads=4",
          "per-crate runs bounded at 5m"
        ]
      },
      "codeTasks": [
        "3.1-t1: `make test TEST_TIMEOUT=10m` green (equivalently `timeout 10m cargo test --workspace --all-targets -- --test-threads=4`)",
        "3.1-t2: `make lint` green (cargo fmt --check plus cargo clippy --workspace --all-targets --all-features -- -D warnings)",
        "3.2-m1: MANUAL — sing-box TUN on an IPv6-less host: status never reaches Connected, backend.log has one ' session ' per candidate with no 2 s/4 s respawn pairs, settings.toml last_success unchanged",
        "3.2-m2: MANUAL — xray TUN on a working node: Connected only after the device wait and xray-up succeed, and settings.toml last_success updated"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "The last-success record SHALL change only when a connection succeeds, meaning the connection is reported `Running` after its backend became ready. A connection that reaches only `Starting`, or whose backend exits or times out before it is ready, SHALL NOT change it. Saving settings from the preferences dialog SHALL NOT modify it.",
      "tests": [
        "crates/ui/src/app.rs::tests::records_last_success_only_on_running",
        "crates/ui/src/connection.rs::tests::two_candidates_first_exits_before_ready"
      ]
    },
    {
      "shall": "- **THEN** the persisted last-success record SHALL still name node B",
      "tests": [
        "crates/ui/src/app.rs::tests::records_last_success_only_on_running"
      ]
    },
    {
      "shall": "- **THEN** the persisted last-success record SHALL still name node A",
      "tests": [
        "crates/ui/src/app.rs::tests::records_last_success_only_on_running"
      ]
    },
    {
      "shall": "- **THEN** the last-success record SHALL change only when the respawned backend is reported `Running`, and SHALL NOT change on the intermediate `Starting`",
      "tests": [
        "crates/ui/src/app.rs::tests::records_last_success_only_on_running"
      ]
    },
    {
      "shall": "The system SHALL report a backend start, and an in-place respawn, as `Running` only after the backend is ready: its local SOCKS (or mixed) inbound accepts a TCP connection and the process is still running at least one second after it was spawned. An unspecified listen address SHALL be probed on the loopback address of the same family. For xray in TUN mode, readiness SHALL be checked after the TUN device has appeared and the route helper has programmed the routes. When the backend does not become ready within 15 seconds, the system SHALL stop it and fail the start with an error naming the probed address and the timeout. When the backend exits before it is ready during the initial start of a candidate, the start SHALL fail with an error stating the exit code or signal and the backend's last output line; the system SHALL NOT respawn it and SHALL NOT record a crash.",
      "tests": [
        "crates/process/src/manager.rs::tests::ready_probe_accepts_live_backend",
        "crates/process/src/manager.rs::tests::ready_timeout_stops_the_backend",
        "crates/process/src/manager.rs::tests::exit_before_ready_carries_last_output",
        "crates/core/src/models/settings.rs::tests::local_endpoint_maps_unspecified_to_loopback"
      ]
    },
    {
      "shall": "- **THEN** the connection SHALL NOT be reported `Running`, no respawn SHALL be attempted for that candidate, and the start SHALL fail with a reason containing the exit code and the last output line",
      "tests": [
        "crates/process/src/manager.rs::tests::startup_failure_writes_one_session_and_no_crash",
        "crates/process/src/manager.rs::tests::exit_before_ready_carries_last_output"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL report `Running` with the connection metadata",
      "tests": [
        "crates/process/src/manager.rs::tests::ready_probe_accepts_live_backend",
        "crates/ui/src/connection.rs::tests::live_connect_writes_backend_diagnostics"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL stop the backend and fail the start with an error naming the address and the timeout",
      "tests": [
        "crates/process/src/manager.rs::tests::ready_timeout_stops_the_backend"
      ]
    },
    {
      "shall": "- **THEN** the connection SHALL try the next candidate without waiting for a crash-restart delay",
      "tests": [
        "crates/ui/src/connection.rs::tests::two_candidates_first_exits_before_ready",
        "crates/ui/src/connection.rs::tests::last_candidate_failure_reports_one_error"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL wait for the TUN device and run the route helper before checking readiness, and SHALL report `Running` only after all three succeed",
      "tests": [
        "MANUAL task 3.2-m2: xray TUN on a working node — Connected only after the device wait and xray-up succeed"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL stop the backend, release TUN routing state, and report `Stopped`",
      "tests": [
        "crates/process/src/manager.rs::tests::ready_timeout_stops_the_backend",
        "crates/process/src/manager.rs::tests::failed_respawn_is_retried",
        "crates/ui/src/connection.rs::tests::stop_halts_every_forwarder"
      ]
    },
    {
      "shall": "The system SHALL detect unexpected exits of a backend that has been reported `Running` and restart it automatically within a bounded budget. An exit before the backend was first ready is a startup failure, not a crash, and SHALL NOT be restarted in place. A respawn that fails to start, including one that exits or times out before it is ready, SHALL count as a crash and SHALL be retried while the budget allows. An in-place respawn SHALL reuse the pre-launch validation of the start it replaces, relaunching the same binary with the same config without re-probing the version, the capabilities, or the config. For xray in TUN mode the respawn SHALL NOT remove the session's routing state.",
      "tests": [
        "crates/process/src/manager.rs::tests::respawn_readiness_failure_exhausts_crash_budget",
        "crates/process/src/manager.rs::tests::startup_failure_writes_one_session_and_no_crash",
        "crates/process/src/manager.rs::tests::respawn_skips_preflight"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL wait 2 seconds and attempt to restart automatically",
      "tests": [
        "crates/process/src/manager.rs::tests::failed_respawn_is_retried"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL transition to Error state instead of restarting. Any exit while the backend is expected to be running counts as a crash, including a signal death (OOM, segfault, external kill); a requested stop moves the state to `Stopping` first and never reaches crash handling.",
      "tests": [
        "crates/process/src/manager.rs::tests::respawn_readiness_failure_exhausts_crash_budget",
        "crates/process/src/manager.rs::tests::respawn_budget_exhaustion_errors"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL NOT wait, SHALL NOT respawn, SHALL NOT count a crash, and SHALL fail the start with the exit reason",
      "tests": [
        "crates/process/src/manager.rs::tests::startup_failure_writes_one_session_and_no_crash"
      ]
    },
    {
      "shall": "- **THEN** the failure SHALL be recorded as a crash and the system SHALL wait and respawn again instead of transitioning to Error",
      "tests": [
        "crates/process/src/manager.rs::tests::respawn_readiness_failure_exhausts_crash_budget"
      ]
    },
    {
      "shall": "- **THEN** it SHALL relaunch without re-running the backend version probe, the capability probe, or the backend's config check",
      "tests": [
        "crates/process/src/manager.rs::tests::respawn_skips_preflight"
      ]
    },
    {
      "shall": "- **THEN** the policy rules installed for the session SHALL remain in place from the exit until the respawned backend's routes are programmed",
      "tests": [
        "MANUAL task 3.2-m2: xray TUN respawn keeps the session's policy rules"
      ]
    }
  ],
  "testHarness": [
    "write_script(dir, body) — crates/process/src/manager.rs — an executable /bin/sh stub named `backend` (0o755) in dir",
    "manager_for(&TempDir, script_body) — crates/process/src/manager.rs — ProcessManager over the stub backend, a `{}` config.json and backend.pid in the temp dir",
    "VERSION_STUB / SINGBOX_VERSION_STUB — crates/process/src/manager.rs — shell prefixes answering `version` as Xray 26.3.27 / sing-box 1.13.0",
    "backend_log(dir) — crates/process/src/manager.rs — Arc<RotatingFileWriter> on dir/backend.log with DEFAULT_MAX_BYTES",
    "stub_helper(dir, body) — crates/process/src/manager.rs — executable netctl stub that appends $1 to dir/calls; returns (helper, calls)",
    "xray_on_lo(helper) — crates/process/src/manager.rs — TunRuntime on iface lo pointing at the stub helper",
    "crashing_backend(dir, crashes) — crates/process/src/manager.rs — script body that exits 1 for the first `crashes` runs (counted in dir/runs) then `exec sleep 30`",
    "read_lines(path) / wait_for_lines(path, n) — crates/process/src/manager.rs — line reader and a 2 s poll-until-n-lines await over the backend log or a marker file",
    "drain_states(&mut broadcast::Receiver<ProcessEvent>) — crates/process/src/manager.rs — Vec<ProcessState> of the StateChanged events queued so far",
    "contains_line(&mgr, content) — crates/process/src/manager.rs — predicate over the last 10 log-buffer lines",
    "HostProbe { getcap, helper } — crates/process/src/manager.rs — pinned host facts for the TUN preflight; with_host_probe also repoints the runtime helper",
    "fake_sleeper_binary(dir) — crates/process/src/probe.rs — a stub that stays alive and never opens a port; paired with a bind-then-drop TcpListener to get a free loopback port",
    "Stub { _tmp, paths, binary } / stub(script) — crates/ui/src/connection.rs — temp AppPaths (AppProfile::Test) with ensure_dirs plus an executable /bin/sh backend stub",
    "request(&Stub, settings, candidates) — crates/ui/src/connection.rs — a full ConnectionRequest (ConfigWriter, pid path, geodata dir, generation 7, host_has_ipv6 true)",
    "connect / connect_with(&Stub, settings, candidates, configure) — crates/ui/src/connection.rs — spawns the connection task and returns (ConnectionHandle, relm4::Receiver<AppMsg>); configure hooks each candidate's manager",
    "capless_probe(mgr) — crates/ui/src/connection.rs — configure hook attaching HostProbe { getcap: /bin/true, helper: /bin/true }",
    "candidate(address) / xhttp_candidate(address) / node(address) — crates/ui/src/connection.rs — ConnectionCandidate over a Shadowsocks node on port 8388 (or a VLESS/xhttp node on 443)",
    "singbox_settings() / tun_settings() / v2ray_settings(socks, http, listen, tun) — crates/ui/src/connection.rs — AppSettings variants; v2ray_settings is the only one that sets listen_address and socks_port",
    "next_state(&rx) / drain(&rx) / assert_nothing_after_terminal(&rx) — crates/ui/src/connection.rs — 20 s-bounded readers over AppMsg: next state, terminal+log-lines, and the no-message-after-terminal assertion",
    "attempts(marker) — crates/ui/src/connection.rs — count of lines the stub appended, one per candidate attempt",
    "paths_with_marker(&TempDir) / stub_helper(&TempDir, body) — crates/ui/src/app.rs — test AppPaths with a saved TunSession marker, and an executable netctl stub (app.rs tests are otherwise pure)"
  ],
  "floor": "make test TEST_TIMEOUT=10m (timeout 10m cargo test --workspace --all-targets -- --test-threads=4) and make lint (cargo fmt -- --check; cargo clippy --workspace --all-targets --all-features -- -D warnings), both green; plus the two manual live checks of task 3.2.",
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
