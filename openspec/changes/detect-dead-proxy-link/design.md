## Context

- Backend output: `crates/process/src/manager.rs:557-595` reads stdout/stderr lines into the log buffer, `backend.log` and the log broadcast; `crates/ui/src/connection.rs:256-269` forwards them to the app as `AppMsg::ProcessLogLine(generation, line)`. Health is not derived from them anywhere.
- `backend.log`, xray 26.9.9, session `2026-09-14T07:44:31` → `2026-09-15T05:03:52`: `[Error] transport/internet/websocket: failed to dial to ap2.directly.chat:443 > tls: failed to verify certificate: x509: certificate has expired or is not yet valid` from 04:26 to 05:03 UTC (tens to hundreds per minute), all for one host.
- Same files: 1586 `[Error] app/dns: failed to retrieve response for <name> > Post "https://<server>/dns-query": …` (1581 of them on 2026-09-15, almost all `context deadline exceeded`); 3044 `[Error] proxy/tun: connection reset by peer` and 2520 `proxy/tun: connection was refused` lines with no destination. Success lines are `from <src> accepted <dst> [in >> out]`, which record routing, not a completed exchange.
- No sing-box runtime outbound error lines exist in the retained logs, so no sing-box format can be verified; v2ray-core's `app/dns` wording is likewise unverified.
- Inbounds: every generator emits an HTTP inbound on `listen_address:http_port` next to the socks/mixed one, TUN or not (`crates/core/src/config/singbox.rs:162-176`, `crates/core/src/config/v2ray.rs:155-169`).
- Real Delay (`crates/subscription/src/real_delay.rs:66-120`) probes through a separate ephemeral backend, not the live session. Its settings (`RealDelaySettings { test_url, timeout_ms, … }`, default `https://www.gstatic.com/generate_204`, 5000 ms, `crates/core/src/models/settings.rs:115-133`) are reusable. The workspace `reqwest` has no `socks` feature (`Cargo.toml:29`).
- Direct connections are tracked as `SessionTarget` in `crates/ui/src/app.rs:1556-1560`; the auto-reconnect budget is `MAX_AUTO_RECONNECTS` 3 (`:35`). Desktop notifications are sent only from `crates/tray/src/notification.rs:25-42` on state changes.

## Goals / Non-Goals

**Goals:**
- A running session whose proxy path is dead is shown as such within about two minutes.
- No false alarm from destination-less tunnel noise.

**Non-Goals:**
- Diagnosing why the path is dead (certificate, DNS, blocking); the backend log already says.
- Health for v2ray/sing-box DNS failures, or log-based outbound classification for any backend.
- Automatic failover by default.

## Decisions

- **Active probe through the live HTTP inbound as the health source.** Every `PROBE_INTERVAL` (30 s, first one 10 s after `Running`), one `GET` of `real_delay.test_url` with `reqwest::Proxy::all("http://<listen>:<http_port>")`, total timeout `real_delay.timeout_ms`. Success is a response with status below 400; any transport error, TLS error or timeout is a failure. `FAILURE_THRESHOLD` 3 consecutive failures → unhealthy; one success → healthy. Only a completed HTTPS exchange through the backend proves the path, which no log line does. Alternatives rejected: log classification as the primary signal (successes are unobservable, formats differ per backend and version, `proxy/tun` noise carries no destination); the socks inbound (needs a new `reqwest` feature); the Real Delay ephemeral backend (tests a different process and is serialized behind a global lock).
- **Probe lives in `v2ray-rs-subscription`** (`health.rs`, `probe_via_http_proxy(proxy_addr, url, timeout) -> Result<(), String>`), next to the other `reqwest` code; the ui crate has no `reqwest` dependency.
- **Monitor lifecycle follows the candidate.** Spawned in `connection.rs` after `Running` is reported, halted with the state and log forwarders on exit, failover or Stop; paused while the manager is `Starting` for a respawn (state receiver) and reset when it is `Running` again. Emits `AppMsg::ConnectionHealth(generation, Health)` only on transitions. Unspecified listen addresses are probed on loopback.
- **xray DNS signal from the log stream.** Lines containing `app/dns: failed to retrieve response` are counted in a 60 s sliding window; ≥ 20 sets `dns_failing`, and 60 s with none clears it. Counted only when the backend is xray. It is a status hint, never a failover trigger, because a failing DNS server choice is a settings problem, not a node problem. `proxy/tun:` lines are never counted.
- **Status and notifications.** Unhealthy while `Running`: primary label `Proxy not responding`, details unchanged plus `· last probe: <reason>`. DNS failing while healthy: primary `Connected`, details prefixed `DNS via proxy failing · `. Entering unhealthy toasts once per streak and, when notifications are enabled, sends a desktop notification through a new `TrayHandle` notify entry point; recovery only updates the status. Health never changes `ProcessState`, so tray icon, terminal-state and reconnect logic are untouched.
- **Opt-in failover.** `HealthCheckSettings { enabled: true, failover: false }` under `AppSettings.health_check` with serde defaults; not part of the runtime snapshot (does not change the config). When `failover` is on, the session has no direct `SessionTarget`, and it turns unhealthy: disconnect and reconnect with `ConnectOrigin::AutoReconnect`, planning with the unhealthy node removed from the candidates for that one attempt. A separate `health_failovers` counter caps consecutive health failovers at `MAX_AUTO_RECONNECTS` (3); it is reset by a successful probe or a user action, not by `Running`, because each failover reaches `Running` and `cancel_auto_reconnect` (`app.rs:1247`) would otherwise refill the budget forever. Direct connections only show the status (connection-auto-resolve "Direct connection to a chosen node").

## Risks / Trade-offs

- [Test URL matched by a Direct or Block routing rule] → the probe measures the wrong path: Direct gives false healthy (status quo), Block gives false unhealthy. Mitigation: failure reason shown in details; checks can be disabled. Forcing probe traffic to the proxy outbound would need a dedicated inbound in three generators — deferred.
- [Local network down makes every node unhealthy] → failover is off by default and bounded by the auto-reconnect budget when on.
- [Probe traffic every 30 s] → one small HTTPS request per interval through the user's proxy.
- [Probe during xray TUN] → it uses the loopback HTTP inbound, not the TUN device, so it does not trigger the TUN accept-path crash (Xray-core #6364) that suppressed TCP pings under TUN.
- [DNS threshold wrong for heavy browsing] → the 1581 failures above were sustained; 20 per minute leaves headroom for sporadic timeouts. Constant in one place.

## Implementation plan

Tier `heavy`, mode `existing-service-strict`, lenses `spec`, `quality`, `arch`. Planned at `1d6a0fb8`.

Rust has no red-stage test-writer agent, so every seam carries `NO-RED-WAIVER` / `NO-TESTER-WAIVER` and its tests are the first `codeTasks` of the chunk that owns them.

This change lands last in sprint `trustworthy-connection-state`. It depends on `AppSettings::local_endpoint` from `confirm-backend-ready-before-running` and on the `StatusView` struct and the halt-set helper from the two changes before it; each dependency carries a fallback if it is absent.


### `h1` — tasks 1.1, 1.2 — seam `settings-and-probe`

- order: parallel, shard `core-sub`; coder `rust-coder`; packages `v2ray-rs-core`, `v2ray-rs-subscription`
- sites:
  - `crates/core/src/models/settings.rs` · `HealthCheckSettings (new struct)` · anchor `impl Default for RealDelaySettings {` — new `HealthCheckSettings { enabled: bool, failover: bool }` with Debug/Clone/PartialEq/Eq/Serialize/Deserialize and a manual Default (enabled true, failover false), placed next to RealDelaySettings
  - `crates/core/src/models/settings.rs` · `AppSettings` · anchor `pub real_delay: RealDelaySettings,` — add `#[serde(default)] pub health_check: HealthCheckSettings,` after this field
  - `crates/core/src/models/settings.rs` · `impl Default for AppSettings` · anchor `real_delay: RealDelaySettings::default(),` — add `health_check: HealthCheckSettings::default(),`
  - `crates/core/src/models/settings.rs` · `mod tests` · anchor `fn test_legacy_settings_toml_missing_real_delay_defaults() {` — clone this test shape for health_check: legacy TOML without the table loads enabled=true/failover=false, plus a TOML round-trip test like test_real_delay_settings_round_trip
  - `crates/core/src/models/mod.rs` · `pub use settings::*` · anchor `pub use settings::{` — add HealthCheckSettings to this explicit re-export list
  - `crates/subscription/src/health.rs` · `probe_via_http_proxy (new file)` · anchor `` — NEW FILE: `pub async fn probe_via_http_proxy(proxy: SocketAddr, url: &str, timeout: Duration) -> Result<(), String>`; reqwest::Client::builder().proxy(reqwest::Proxy::all(format!("http://{proxy}"))).timeout(timeout).build(); status < 400 -> Ok, else Err(status text); transport error -> Err(e.to_string()) (reqwest timeout errors stringify containing `operation timed out`)
  - `crates/subscription/src/lib.rs` · `module list` · anchor `pub(crate) mod real_delay;` — add `pub(crate) mod health;` in alphabetical position (after fetch)
  - `crates/subscription/src/lib.rs` · `re-exports` · anchor `pub use real_delay::{RealDelayReport, measure_real_delay};` — add `pub use health::probe_via_http_proxy;` — required, the ui crate calls it and every module here is pub(crate)
- work, in order:
  - TEST FIRST: in crates/core/src/models/settings.rs `mod tests`, add `test_legacy_settings_toml_missing_health_check_defaults` (TOML string identical to the one at anchor 'fn test_legacy_settings_toml_missing_real_delay_defaults() {', asserting settings.health_check.enabled == true and settings.health_check.failover == false) and `test_health_check_settings_round_trip` (AppSettings with `health_check: HealthCheckSettings { enabled: false, failover: true }`, `..AppSettings::default()`, toml::to_string -> from_str, assert_eq on the whole AppSettings).
  - TEST FIRST: create crates/subscription/src/health.rs with `mod tests` holding `probe_succeeds_on_204`, `probe_fails_on_502`, `probe_fails_on_timeout` (asserting the Err string contains `timed out`), each using a local tokio::net::TcpListener and an http:// URL.
  - Add `HealthCheckSettings { pub enabled: bool, pub failover: bool }` to crates/core/src/models/settings.rs next to RealDelaySettings (anchor 'impl Default for RealDelaySettings {'), deriving Debug, Clone, PartialEq, Eq, Serialize, Deserialize, with a manual `impl Default` setting enabled = true, failover = false.
  - Add `#[serde(default)] pub health_check: HealthCheckSettings,` immediately after the anchor '    pub real_delay: RealDelaySettings,' in AppSettings, and `health_check: HealthCheckSettings::default(),` after the anchor '            real_delay: RealDelaySettings::default(),' in impl Default.
  - Add `HealthCheckSettings` to the explicit re-export list at anchor 'pub use settings::{' in crates/core/src/models/mod.rs.
  - Implement `pub async fn probe_via_http_proxy(proxy: SocketAddr, url: &str, timeout: Duration) -> Result<(), String>` in crates/subscription/src/health.rs: build a per-call client with `reqwest::Client::builder().proxy(reqwest::Proxy::all(format!("http://{proxy}")).map_err(|e| e.to_string())?).timeout(timeout).build()`, GET the url, return Ok(()) when `response.status().as_u16() < 400`, otherwise `Err(status.to_string())`; on a transport error return `Err("timed out".to_string())` when `err.is_timeout()` and `Err(err.to_string())` otherwise.
  - Register the module: add `pub(crate) mod health;` after the anchor 'pub(crate) mod fetch;' in crates/subscription/src/lib.rs, and `pub use health::probe_via_http_proxy;` beside the anchor 'pub use real_delay::{RealDelayReport, measure_real_delay};'.
- verify: `make test-core && make test-subscription && make lint`

### `h2` — tasks 2.1, 2.2 — seam `ui-health-trackers`

- order: after `h1` (compile dependency, no shared package); coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/health.rs` · `HealthTracker, Health (new file)` · anchor `` — NEW FILE (recommended over growing connection.rs): pure `Health` enum {Healthy, Unhealthy(String)}, `HealthTracker::record(Result<(), String>) -> Option<Health>` returning a transition only, `reset()`; FAILURE_THRESHOLD = 3 const here
  - `crates/ui/src/lib.rs` · `module list` · anchor `pub(crate) mod failure_streak;` — add `pub(crate) mod health;` after this line
  - `crates/ui/src/health.rs` · `DnsFailureWindow` · anchor `` — same new file: `observe(&mut self, line: &str, now: Instant)` counts only lines containing `app/dns: failed to retrieve response`; `is_failing(&self, now: Instant) -> bool` true at >= 20 within 60 s, clears 60 s after the last counted line; constants DNS_FAILURE_THRESHOLD/DNS_WINDOW local
- work, in order:
  - TEST FIRST: create the `mod tests` in crates/ui/src/health.rs with `two_failures_report_nothing_third_reports_unhealthy`, `further_failures_stay_quiet`, `success_after_unhealthy_reports_healthy`, `first_success_of_a_session_reports_healthy`, `success_between_failures_resets_the_streak`, `reset_clears_the_streak_and_reports_nothing`, `dns_window_marks_failing_at_twenty_within_sixty_seconds`, `dns_window_clears_sixty_seconds_after_the_last_line`, `dns_window_ignores_tun_noise` (500 lines).
  - Create crates/ui/src/health.rs with `#[derive(Debug, Clone, PartialEq, Eq)] pub(crate) enum Health { Healthy, Unhealthy(String) }`, `const FAILURE_THRESHOLD: u32 = 3;`, and `pub(crate) struct HealthTracker { failures: u32, last_reported: Option<Health> }` exposing `new()`, `record(&mut self, outcome: Result<(), String>) -> Option<Health>` and `reset(&mut self)` per the transition table.
  - Add `pub(crate) struct DnsFailureWindow` to the same file: `observe(&mut self, line: &str, now: Instant)` pushes `now` only when `line.contains("app/dns: failed to retrieve response")` and drops entries older than DNS_WINDOW; `is_failing(&self, now: Instant) -> bool` counts entries within DNS_WINDOW of `now` and compares against DNS_FAILURE_THRESHOLD. Constants `DNS_FAILURE_THRESHOLD: usize = 20` and `DNS_WINDOW: Duration = Duration::from_secs(60)` live in this file.
  - Register the module: add `pub(crate) mod health;` after the anchor 'pub(crate) mod failure_streak;' in crates/ui/src/lib.rs.
- verify: `make test-ui && make lint`

### `h3` — tasks 4.4 — seam `preferences-page`

- order: after `h1` (compile dependency, no shared package); coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/preferences/network.rs` · `build_network_page (rows)` · anchor `let real_delay_use_for_lowest_row = adw::SwitchRow::builder()` — add a `Connection health` PreferencesGroup after real_delay_group with two adw::SwitchRow rows: "Check connection health" (health_check.enabled) and "Reconnect when the proxy stops responding" (health_check.failover); build them before `drop(s);`
  - `crates/ui/src/preferences/network.rs` · `build_network_page (handlers)` · anchor `real_delay_use_for_lowest_row.connect_active_notify(move |row| {` — copy this block twice: `st.borrow_mut().health_check.enabled/.failover = row.is_active(); emit(&st, &cb);`
- work, in order:
  - In crates/ui/src/preferences/network.rs, before the anchor '    drop(s);', add a `adw::PreferencesGroup` titled 'Connection health' holding `health_check_enabled_row` (adw::SwitchRow, title 'Check connection health', .active(s.health_check.enabled)) and `health_check_failover_row` (title 'Reconnect when the proxy stops responding', .active(s.health_check.failover)); `page.add(&health_check_group);`.
  - After the anchor '        real_delay_use_for_lowest_row.connect_active_notify(move |row| {' block, add two identically shaped blocks: `st.borrow_mut().health_check.enabled = row.is_active(); emit(&st, &cb);` and the same for `.failover`.
  - Do not touch crates/ui/src/app.rs in this seam.
- verify: `make test-ui && make lint`

### `h4` — tasks 3.1, 3.2 — seam `ui-wiring`

- order: after `h2` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/connection.rs` · `spawn_with candidate loop` · anchor `report(ProcessState::Running, Some(meta.clone()));` — after reporting Running, spawn the health monitor when settings.health_check.enabled; probe addr = listen_address (unspecified -> loopback) + settings.http_port, url/timeout from settings.real_delay (timeout_ms is u32 millis)
  - `crates/ui/src/connection.rs` · `state_forwarder / monitor pause-reset` · anchor `_ = mgr.wait_and_handle_exit() => {` — monitor consumes a second `mgr.subscribe()` receiver so Starting pauses probing and Running resets the tracker; keep the existing forwarder untouched
  - `crates/ui/src/connection.rs` · `exit paths (halt)` · anchor `halt(log_forwarder).await;` — halt the monitor task alongside the forwarders here AND on every early `return` in the supervise loop / Stop branches, so no probe traffic survives handle.stop()
  - `crates/ui/src/app.rs` · `AppMsg` · anchor `ProcessLogLine(u64, String),` — add `ConnectionHealth(u64, Health)` variant (Health must derive Debug for the AppMsg derive)
  - `crates/ui/src/connection.rs` · `log_forwarder` · anchor `Ok(ProcessEvent::LogLine(line)) => {` — when settings.backend.backend_type == BackendType::Xray, feed line.content into DnsFailureWindow and emit AppMsg::DnsHealth(generation, bool) on change; other backends unchanged
  - `crates/ui/src/connection.rs` · `log_sender capture` · anchor `let log_sender = sender.clone();` — move the backend type (Copy) into the forwarder task alongside log_sender
  - `crates/ui/src/app.rs` · `AppMsg` · anchor `ProcessLogLine(u64, String),` — add `DnsHealth(u64, bool)` variant
- work, in order:
  - TEST FIRST in crates/ui/src/connection.rs `mod tests`: `health_monitor_reports_unhealthy_then_healthy` (fake proxy listener; probes start failing -> exactly one AppMsg::ConnectionHealth(GENERATION, Health::Unhealthy(_)); restored -> exactly one Health::Healthy), `health_monitor_is_not_spawned_when_disabled` (health_check.enabled = false -> listener records zero accepts), `health_monitor_stops_with_the_connection` (handle.stop(); assert_nothing_after_terminal plus zero further accepts on the listener), `xray_dns_burst_reports_dns_health` (stub prints 25 matching lines -> exactly one AppMsg::DnsHealth(GENERATION, true)), `singbox_dns_burst_reports_nothing` (same stub output under singbox_settings() -> no AppMsg::DnsHealth).
  - Add `HealthTiming { pub initial_delay: Duration, pub interval: Duration }` to crates/ui/src/connection.rs with a Default of 10 s / 30 s, add `pub health_timing: HealthTiming` to ConnectionRequest, destructure it in spawn_with beside `host_has_ipv6`, set `HealthTiming::default()` at the app.rs construction site, and add a `connect_with_health` test helper that overrides it.
  - Replace the ad-hoc halts in the supervision loop with one helper that halts every spawned task (state_forwarder, log_forwarder, and the monitor when present) and call it on all three post-Running exit paths: the Some(ConnectionCmd::Stop) arm, the `_ =>` arm after mgr.wait_and_handle_exit(), and the fall-through after the loop breaks.
  - Spawn the health monitor after the anchor '                    report(ProcessState::Running, Some(meta.clone()));' when effective_settings.health_check.enabled: it takes a second `mgr.subscribe()`, sleeps HealthTiming::initial_delay, then loops on a tokio::select! over the state receiver and an interval tick; Starting pauses probing and calls HealthTracker::reset(), Running resumes, Lagged continues, Closed breaks. Each tick calls v2ray_rs_subscription::probe_via_http_proxy(addr, &settings.real_delay.test_url, timeout) and emits AppMsg::ConnectionHealth(generation, health) for every Some(_) the tracker returns.
  - Compute the probe address from the shared helper: `effective_settings.local_endpoint(effective_settings.http_port)` when confirm-backend-ready-before-running is already merged; otherwise add that helper to crates/core/src/models/settings.rs beside the anchor '    pub fn validate_listen_address(addr: &str) -> Result<(), ValidationError> {' with its four unit cases (127.0.0.1, 0.0.0.0 -> 127.0.0.1, :: -> ::1, 192.168.1.10).
  - Feed the DNS window from the log forwarder: move the Copy backend_type into the task at the anchor '            let log_sender = sender.clone();', and inside the arm at anchor '                        Ok(ProcessEvent::LogLine(line)) => {' call DnsFailureWindow::observe when the backend is BackendType::Xray, emitting AppMsg::DnsHealth(generation, flag) only when is_failing flips.
  - Add `ConnectionHealth(u64, Health)` and `DnsHealth(u64, bool)` to AppMsg after the anchor '    ProcessLogLine(u64, String),'.
  - Add the two `AppMsg::ConnectionHealth` / `AppMsg::DnsHealth` match arms in the SAME chunk that adds the variants — `App::update`'s match at crates/ui/src/app.rs has no wildcard arm, so adding a variant without its arm is E0004 and h4's own `make test-ui` fails. h4 lands them behind the existing `is_current_generation` guard storing the value; h5 fills in the announce/failover behavior.
  - Drive the DNS clear edge from a timer, not from log lines: the window is only evaluated inside the LogLine arm, so once a burst stops no further line arrives, `is_failing` is never re-evaluated and `DnsHealth(_, false)` is never emitted — the 60 s clear requirement would be unimplemented. Re-evaluate on a `tokio::time::interval(DNS_RECHECK_INTERVAL)` tick in the forwarder select (or on the health monitor's own interval tick).
  - TEST FIRST: add `dns_window_clears_after_the_recheck_tick` — DnsFailureWindow takes `now` as a parameter, so drive 20 matching lines then a tick 61 s later and assert is_failing flips false; at the forwarder level ride the existing xray burst test with an injected short recheck interval and assert exactly one `AppMsg::DnsHealth(GENERATION, false)`
- verify: `make test-ui && make lint`

### `h5` — tasks 4.1, 4.3 — seam `ui-wiring`

- order: after `h4` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/app.rs` · `App struct` · anchor `connection_status: Option<ConnectionMetadata>,` — add `health: Option<Health>` and `dns_failing: bool` fields (cleared on any non-Running state)
  - `crates/ui/src/app.rs` · `App::init model literal` · anchor `session_target: None,` — initialize the two new fields in the model literal
  - `crates/ui/src/app.rs` · `status_texts(&StatusView) (shared with log-connection-decisions)` · anchor `fn update_status_labels(&self) {` — Extend the sprint's shared `StatusView { state, prev_state, meta, origin, attempt }` with `health: Option<Health>` and `dns_failing: bool`, and the pure `status_texts(&StatusView) -> (String, String)` with the three Running arms in the sprint's precedence order. `log-connection-decisions` introduces the struct and the function (its tasks.md 4.1 now names `status_texts(&StatusView)`, not `status_primary`); this change lands last and only adds fields. If it is absent, introduce it here with `state`, `meta`, `health`, `dns_failing` and leave room for `prev_state`, `origin`, `attempt`.
  - `crates/ui/src/app.rs` · `status_texts(&StatusView) (shared with log-connection-decisions)` · anchor `("Connected".to_string(), details)` — Extend the sprint's shared `StatusView { state, prev_state, meta, origin, attempt }` with `health: Option<Health>` and `dns_failing: bool`, and the pure `status_texts(&StatusView) -> (String, String)` with the three Running arms in the sprint's precedence order. `log-connection-decisions` introduces the struct and the function (its tasks.md 4.1 now names `status_texts(&StatusView)`, not `status_primary`); this change lands last and only adds fields. If it is absent, introduce it here with `state`, `meta`, `health`, `dns_failing` and leave room for `prev_state`, `origin`, `attempt`.
  - `crates/ui/src/app.rs` · `AppMsg::ConnectionHealth / DnsHealth handlers` · anchor `AppMsg::ProcessLogLine(generation, line) => {` — add handlers next to this one, gated by the same `is_current_generation(generation, self.connection_generation)` guard; store health/dns_failing then update_status_labels()
  - `crates/ui/src/app.rs` · `App::apply_state` · anchor `self.process_state = state.clone();` — clear health and dns_failing whenever the new state is not Running, so Starting cannot show stale health
  - `crates/ui/src/app.rs` · `App struct` · anchor `auto_reconnect_attempts: u32,` — add `health_failovers: u32` and `excluded_node: Option<ConnectionNodeRef>` (one-attempt exclusion); initialize both in the model literal
  - `crates/ui/src/app.rs` · `AppMsg::Connect handler` · anchor `let candidates = planner.plan(&subscriptions, &manual_nodes);` — after planning, drop `self.excluded_node.take()` from candidates for this attempt only; empty-after-exclusion falls through to the existing empty-candidates toast
  - `crates/ui/src/app.rs` · `health failover trigger` · anchor `AppMsg::ProcessLogLine(generation, line) => {` — in the ConnectionHealth handler added next to this arm: on Unhealthy, if health_failover_allowed(settings.health_check.failover, self.session_target, self.health_failovers) set excluded_node from connection_status.node_ref, increment health_failovers, set reconnect_pending and dispatch Disconnect then Connect(ConnectOrigin::AutoReconnect)
  - `crates/ui/src/app.rs` · `health_failover_allowed / exclude_candidate (new pure fns)` · anchor `struct SessionTarget {` — add the two pure helpers beside SessionTarget: allowed only when enabled, session_target is None (direct target -> false) and count < MAX_AUTO_RECONNECTS
  - `crates/ui/src/app.rs` · `budget constant` · anchor `const MAX_AUTO_RECONNECTS: u32 = 3;` — reuse this cap for health failovers; do NOT reuse auto_reconnect_attempts (cancel_auto_reconnect runs on every Running and would refill it)
  - `crates/ui/src/app.rs` · `counter reset` · anchor `fn cancel_auto_reconnect(&mut self) {` — do NOT reset health_failovers here; reset it explicitly in the AppMsg::Connect / ConnectToNode / Disconnect arms (user actions) and on a Healthy transition
  - `crates/ui/src/app.rs` · `AppMsg::Connect user reset` · anchor `AppMsg::Connect(origin) => {` — reset health_failovers and clear excluded_node when cancels_auto_reconnect(origin) is true (User/Restart), not for AutoReconnect
  - `crates/ui/src/app.rs` · `AppMsg::Disconnect` · anchor `AppMsg::Disconnect => {` — reset health_failovers and clear excluded_node
  - `crates/ui/src/app.rs` · `AppMsg::ConnectToNode` · anchor `AppMsg::ConnectToNode(target, origin) => {` — reset health_failovers on a user direct connect (the session_target it sets already blocks failover)
- work, in order:
  - TEST FIRST in crates/ui/src/app.rs `mod tests`: `status_texts_unhealthy_says_proxy_not_responding`, `status_texts_dns_failing_prefixes_details`, `status_texts_healthy_matches_todays_connected_text`, `status_texts_starting_ignores_stale_health`, `health_failover_allowed_rejects_direct_target`, `health_failover_allowed_rejects_exhausted_budget`, `health_failover_allowed_rejects_disabled_preference`, `exclude_candidate_drops_only_the_named_node`.
  - Add `health: Option<Health>`, `dns_failing: bool`, `health_failovers: u32`, `excluded_node: Option<ConnectionNodeRef>` and `health_reconnect_pending: bool` to the App struct and to the model literal at the anchor '            session_target: None,'.
  - Extract the body of `fn update_status_labels(&self)` into a pure function over the sprint's StatusView struct, adding the `health` and `dns_failing` fields to it (the 4.1 site states the fallback if StatusView is absent), and implement the three Running arms in the sprint's precedence order.
  - Clear health and dns_failing in App::apply_state whenever the new state is not ProcessState::Running, at the anchor '        self.process_state = state.clone();'.
  - Extend the two message handlers h4 created beside the anchor '            AppMsg::ProcessLogLine(generation, line) => {', both behind the existing is_current_generation guard; the ConnectionHealth handler stores the health, resets health_failovers on Healthy, calls update_status_labels, and on an Unhealthy transition captures self.connection_status.as_ref().map(|m| m.node_ref) BEFORE deciding on failover.
  - Add `fn health_failover_allowed(enabled: bool, session_target: Option<SessionTarget>, count: u32) -> bool` and `fn exclude_candidate(candidates: Vec<ConnectionCandidate>, excluded: Option<ConnectionNodeRef>) -> Vec<ConnectionCandidate>` beside the anchor 'struct SessionTarget {'.
  - Wire the failover: on an armed Unhealthy set excluded_node, increment health_failovers, set health_reconnect_pending and dispatch AppMsg::Disconnect; in the terminal branch of AppMsg::ProcessStateConnection check health_reconnect_pending BEFORE the reconnect_after_stop check and dispatch AppMsg::Connect(ConnectOrigin::AutoReconnect); apply exclude_candidate after the anchor '                let candidates = planner.plan(&subscriptions, &manual_nodes);'; reset health_failovers and clear excluded_node in the Connect/ConnectToNode arms when cancels_auto_reconnect(origin) is true and in the Disconnect arm ONLY when `!self.health_reconnect_pending` (the failover issues its own Disconnect and must keep its exclusion and budget).
  - Gate the Disconnect-arm reset on `!self.health_reconnect_pending`: the failover dispatches AppMsg::Disconnect itself, and the existing arm clears `excluded_node` and `health_failovers` (and calls cancel_auto_reconnect) on every Disconnect — so without the carve-out the exclusion is cleared before AppMsg::Connect filters the plan, the budget never accumulates, and the same dead node is re-picked without bound.
- verify: `make test-ui && make lint`

### `h6` — tasks 4.2 — seam `tray-notify`

- order: after `h5` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-tray`, `v2ray-rs-ui`
- sites:
  - `crates/ui/src/app.rs` · `should_announce (new pure fn)` · anchor `fn active_nodes_available(subscriptions: &[Subscription], manual_nodes: &[ManualNode]) -> bool {` — add `fn should_announce(prev: Option<&Health>, next: &Health) -> bool` near the other pure helpers; only a Healthy/None -> Unhealthy edge announces
  - `crates/ui/src/app.rs` · `toast on unhealthy` · anchor `fn show_toast(&self, msg: &str) {` — reuse show_toast for `Proxy not responding: <reason>`; when settings.notifications_enabled also call the new tray notify entry point through TRAY_HANDLE
  - `crates/ui/src/app.rs` · `tray handle access` · anchor `fn update_tray_notification_setting(enabled: bool) {` — add a sibling free fn that locks TRAY_HANDLE and calls the new TrayHandle notify method (same `if let Ok(mut guard) ... and let Some(handle)` shape)
  - `crates/tray/src/tray.rs` · `impl TrayHandle` · anchor `pub fn set_notifications_enabled(&mut self, enabled: bool) {` — add `pub fn notify(&self, summary: &str, body: &str)` delegating to the Notifier; keep the enabled gate inside the Notifier
  - `crates/tray/src/notification.rs` · `Notifier` · anchor `fn send(&self, summary: &str, body: &str) {` — make a public wrapper (e.g. `pub fn notify(&self, summary, body)`) that checks `self.enabled` then calls send; `send` itself is already blocking (Notification::show) so the caller should spawn_blocking like TrayService does
- work, in order:
  - TEST FIRST in crates/ui/src/app.rs `mod tests`: `should_announce_fires_once_per_unhealthy_streak` covering all four rows of the transition table.
  - Add `pub fn notify(&self, summary: &str, body: &str)` to Notifier in crates/tray/src/notification.rs, checking `self.enabled` then delegating to the existing private `send`.
  - Add `pub fn notify(&self, summary: &str, body: &str)` to TrayHandle beside the anchor '    pub fn set_notifications_enabled(&mut self, enabled: bool) {' in crates/tray/src/tray.rs, delegating to the Notifier.
  - Add `fn should_announce(prev: Option<&Health>, next: &Health) -> bool` near the anchor 'fn active_nodes_available(subscriptions: &[Subscription], manual_nodes: &[ManualNode]) -> bool {'.
  - Add a free function beside the anchor 'fn update_tray_notification_setting(enabled: bool) {' that clones what it needs out of the TRAY_HANDLE lock, releases the lock, and runs the notify call inside tokio::task::spawn_blocking.
  - In the AppMsg::ConnectionHealth handler added by the ui-wiring seam, call show_toast with `Proxy not responding: <reason>` on an announced edge, and the new notify path additionally when settings.notifications_enabled.
- verify: `make test-tray && make test-ui && make lint`

### `h7` — tasks 5.1, 5.2 — seam `verification`

- order: after `h6` (shared `crates/ui/src`); coder `rust-coder`; packages `v2ray-rs-core`, `v2ray-rs-subscription`, `v2ray-rs-ui`, `v2ray-rs-tray`
- work, in order:
  - Run `TEST_TIMEOUT=10m make test` and `make lint` and fix what they report.
  - Perform the live xray check described in task 5.2 and record the observed status text, toast count and, with failover on, the node the reconnect picked. Report it as a manual result.
- verify: `TEST_TIMEOUT=10m make test && make lint`

### Contracts


**`settings-and-probe`** — tasks 1.1, 1.2
- states: `health_check_defaults`, `health_check_custom`, `probe_ok`, `probe_status_error`, `probe_timeout`, `probe_transport_error`
- transitions:
  - `settings TOML carrying no [health_check] table` → `health_check_defaults` → **set** — anchor 'fn test_legacy_settings_toml_missing_real_delay_defaults() {' — the same shape, cloned for health_check
  - `AppSettings::default()` → `health_check_defaults` → **set** — anchor '            real_delay: RealDelaySettings::default(),' in impl Default for AppSettings
  - `toml::to_string then toml::from_str of an AppSettings with health_check.failover = true` → `health_check_custom` → **no-op** — anchor 'fn test_real_delay_settings_round_trip() {' — asserts full AppSettings equality after the round trip
  - `proxy listener answers 'HTTP/1.1 204 No Content'` → `probe_ok` → **set** — requirement: 'A response with a status below 400 SHALL count as a success'
  - `proxy listener answers 'HTTP/1.1 502 Bad Gateway'` → `probe_status_error` → **set** — requirement: 'A response with a status below 400 SHALL count as a success; a connection, TLS, or protocol error or a timeout SHALL count as a failure'
  - `proxy listener accepts and never writes, probe timeout 300 ms` → `probe_timeout` → **set** — requirement: 'or a timeout SHALL count as a failure'
  - `no listener bound at the proxy address` → `probe_transport_error` → **set** — requirement: 'a connection, TLS, or protocol error ... SHALL count as a failure'
- forbidden:
  - health_check added to RuntimeConfigSnapshot — verified absent from the literal at anchor 'self.runtime_snapshot = Some(RuntimeConfigSnapshot {'; adding it would make check_restart_required raise the restart banner on a toggle, contradicting task 4.4
  - health_check declared before any scalar field of AppSettings — toml::to_string fails with ValueAfterTable; it must sit after 'pub real_delay: RealDelaySettings,', which is today the last field
  - a socks:// proxy URL — the workspace reqwest pin 'reqwest = { version = "0.13", features = ["rustls-no-provider", "stream"], default-features = false }' has no `socks` feature; only http:// proxies are reachable
  - classifying a timeout by matching reqwest::Error's Display text — the probe must branch on err.is_timeout() and emit its own message containing 'timed out'
  - probe_via_http_proxy left crate-private — every module in crates/subscription/src/lib.rs is pub(crate), so without an explicit re-export the ui crate cannot call it
- seeding:
  - health_check_defaults: only via toml::from_str on a TOML string with no [health_check] table, or AppSettings::default(). Never by writing the field on a constructed struct.
  - health_check_custom: only via an AppSettings struct literal with '..AppSettings::default()', mirroring 'fn test_real_delay_settings_round_trip'.
  - probe_ok / probe_status_error / probe_timeout: only by calling probe_via_http_proxy against a tokio::net::TcpListener bound to 127.0.0.1:0 that speaks raw HTTP/1.1, modelled on 'async fn spawn_mock_clash(responses: Vec<Option<u64>>) -> u16' in crates/subscription/src/real_delay.rs. The probe URL in these tests MUST use the http:// scheme: with an https:// URL reqwest sends CONNECT to the proxy, which the raw listener does not implement.
  - probe_transport_error: bind a listener with pick_free_loopback_port()'s pattern, drop it, then probe that port.
- budgets:
  - probe timeout in the timeout test: 300 ms
  - success status threshold: status < 400
  - health_check defaults: enabled = true, failover = false
  - three probe tests total (204, 502, no-answer); a fourth for the unbound port is optional

**`ui-health-trackers`** — tasks 2.1, 2.2
- states: `healthy`, `failing_1`, `failing_2`, `unhealthy`, `dns_clear`, `dns_failing`
- transitions:
  - `record(Err(reason)) with 0 consecutive failures recorded` → `failing_1` → **no-op** — requirement: 'After 3 consecutive failures the session SHALL be unhealthy'
  - `record(Err(reason)) with 1 consecutive failure recorded` → `failing_2` → **no-op** — requirement: 'After 3 consecutive failures the session SHALL be unhealthy'
  - `record(Err(reason)) with 2 consecutive failures recorded` → `unhealthy` → **set** — requirement: 'After 3 consecutive failures the session SHALL be unhealthy' — returns Some(Health::Unhealthy(reason))
  - `record(Err(reason)) while already unhealthy` → `unhealthy` → **no-op** — requirement scenario 'Continued failures stay quiet'
  - `record(Ok(())) while unhealthy` → `healthy` → **set** — requirement: 'the next success SHALL make it healthy' — returns Some(Health::Healthy)
  - `record(Ok(())) while last_reported is None (fresh tracker or after reset)` → `healthy` → **set** — connection-auto-resolve requirement: 'a successful probe ... SHALL reset that count' — without this edge the failover budget can never be reset by a probe; returns Some(Health::Healthy)
  - `record(Ok(())) while last_reported is Some(Health::Healthy)` → `healthy` → **no-op** — requirement scenario 'Isolated failure': one failure then a success leaves the session healthy and reports nothing new
  - `record(Err(_)) after a single earlier failure followed by record(Ok(()))` → `failing_1` → **forced** — requirement: 'After 3 consecutive failures' — the counter is reset by any success
  - `reset()` → `healthy` → **forced** — requirement: 'Probing SHALL pause while the backend is being respawned and restart from a healthy state' — clears the counter and last_reported, returns nothing
  - `observe(line containing 'app/dns: failed to retrieve response', now)` → `dns_clear` → **set** — requirement: 'the system SHALL count backend log lines reporting `app/dns: failed to retrieve response`' — records the timestamp
  - `observe(line containing 'proxy/tun: connection reset by peer', now)` → `dns_clear` → **no-op** — requirement: 'Log lines that carry no destination, such as `proxy/tun: connection reset by peer` and `proxy/tun: connection was refused`, SHALL NOT affect health'
  - `is_failing(now) with 20 or more counted lines whose timestamps are within 60 s of now` → `dns_failing` → **set** — requirement: 'SHALL mark DNS through the proxy as failing when at least 20 such lines occur within 60 seconds'
  - `is_failing(now) 60 s after the newest counted line` → `dns_clear` → **forced** — requirement: 'the mark SHALL clear after 60 seconds without such a line'
- forbidden:
  - reset() returning a transition — the App has already cleared health on the non-Running state that caused the reset, so a Healthy emission there would repaint a Starting status bar
  - DnsFailureWindow reading Instant::now() internally — `now` is a parameter on both observe and is_failing, so tests drive synthetic time
  - DnsFailureWindow counting any line whose text starts with 'proxy/tun:'
  - Health without a Debug derive — AppMsg is declared '#[derive(Debug)]' at anchor 'pub enum AppMsg {' and will not compile otherwise
  - HealthTracker holding a reqwest client, a SocketAddr or any IO — it records outcomes only
- seeding:
  - healthy / failing_1 / failing_2 / unhealthy: only `HealthTracker::new()` followed by `record(Ok(()))` / `record(Err(String))` calls. Never by writing the internal counter.
  - the after-reset healthy state: only `HealthTracker::reset()`.
  - dns_clear / dns_failing: only `DnsFailureWindow::default()` then `observe(line, now)` with Instants built as `base + Duration::from_secs(k)` from one `Instant::now()` captured at the top of the test.
- budgets:
  - FAILURE_THRESHOLD = 3 (consecutive probe failures before Unhealthy)
  - DNS_FAILURE_THRESHOLD = 20 (counted lines)
  - DNS_WINDOW = Duration::from_secs(60)
  - noise test feeds 500 'proxy/tun: connection reset by peer' lines and asserts is_failing stays false
  - DNS_RECHECK_INTERVAL: Duration::from_secs(10) — the timer tick that re-evaluates DnsFailureWindow::is_failing so the 60 s clear edge fires without a further log line

**`preferences-page`** — tasks 4.4
- states: `row_reflects_settings`, `settings_updated`, `restart_banner_quiet`
- transitions:
  - `page built with health_check.enabled = true` → `row_reflects_settings` → **set** — anchor '    let real_delay_use_for_lowest_row = adw::SwitchRow::builder()' — .active(s.real_delay.use_for_lowest_latency) is the exact binding shape
  - `user toggles 'Check connection health'` → `settings_updated` → **set** — anchor '        real_delay_use_for_lowest_row.connect_active_notify(move |row| {' — st.borrow_mut() mutation followed by emit(&st, &cb)
  - `user toggles 'Reconnect when the proxy stops responding'` → `settings_updated` → **set** — same anchor, copied a second time
  - `either toggle while a connection is Running` → `restart_banner_quiet` → **no-op** — anchor 'snapshot.diverges_from(&self.settings, &current_rules, manual_nodes, subscriptions)' — only RuntimeConfigSnapshot fields can raise the banner, and health_check is deliberately not one of them
- forbidden:
  - adding health_check to RuntimeConfigSnapshot (anchor 'self.runtime_snapshot = Some(RuntimeConfigSnapshot {') — it would make each toggle raise the restart banner
  - building the rows after the anchor '    drop(s);' — `s` is the borrow the .active(...) reads come from
- seeding:
  - row_reflects_settings and settings_updated are reachable only through build_network_page with a live adw::PreferencesPage; there is no headless path, so both are proven by the manual step in the verification seam.
- budgets:
  - two rows, one PreferencesGroup, added after the group built at anchor '    let real_delay_group = adw::PreferencesGroup::builder()'
  - zero new unit tests in this seam

**`ui-wiring`** — tasks 3.1, 3.2, 4.1, 4.3
- states: `monitor_absent`, `monitor_probing`, `monitor_paused`, `monitor_halted`, `app_health_none`, `app_health_healthy`, `app_health_unhealthy`, `app_dns_failing`, `failover_armed`, `failover_blocked`
- transitions:
  - `candidate reported Running with settings.health_check.enabled == true` → `monitor_probing` → **set** — anchor '                    report(ProcessState::Running, Some(meta.clone()));' — the monitor is spawned immediately after it, beside the two existing forwarders
  - `candidate reported Running with settings.health_check.enabled == false` → `monitor_absent` → **no-op** — requirement: 'When health checks are disabled, no probe SHALL be sent.'
  - `StateChanged { to: ProcessState::Starting } on the monitor's own receiver` → `monitor_paused` → **forced** — anchor 'let _ = self.state.transition(ProcessState::Starting, self.current_connection.clone());' in handle_unexpected_exit — the respawn relay; the monitor calls HealthTracker::reset() and stops probing
  - `StateChanged { to: ProcessState::Running } on the monitor's own receiver` → `monitor_probing` → **forced** — requirement: 'Probing SHALL pause while the backend is being respawned and restart from a healthy state when it is `Running` again.'
  - `broadcast::error::RecvError::Lagged on the monitor's receiver` → `monitor_probing` → **no-op** — anchor 'Err(broadcast::error::RecvError::Lagged(_)) => continue,' — the existing forwarders' handling; a busy log burst must not kill the monitor
  - `ConnectionCmd::Stop arm of the supervision loop` → `monitor_halted` → **forced** — anchor '                        halt(state_forwarder).await;' inside the Some(ConnectionCmd::Stop) arm — today this path leaks log_forwarder; the replacement halts every spawned task
  - `the '_ =>' arm after mgr.wait_and_handle_exit()` → `monitor_halted` → **forced** — anchor '                                halt(state_forwarder).await;' inside the `_ =>` arm — the second leaking path
  - `supervision loop breaks on ProcessState::Error (candidate give-up)` → `monitor_halted` → **forced** — anchor '            halt(log_forwarder).await;' — the only path that halts both today
  - `HealthTracker::record returns Some(health)` → `app_health_unhealthy or app_health_healthy` → **set** — requirement: 'After 3 consecutive failures the session SHALL be unhealthy' — the monitor emits AppMsg::ConnectionHealth(generation, health)
  - `log line while settings.backend.backend_type == BackendType::Xray and DnsFailureWindow::is_failing flips` → `app_dns_failing` → **set** — anchor '                        Ok(ProcessEvent::LogLine(line)) => {' — emits AppMsg::DnsHealth(generation, bool) beside the existing AppMsg::ProcessLogLine
  - `the same log lines while backend_type is SingBox or V2ray` → `app_dns_failing` → **no-op** — requirement: 'Other backends SHALL NOT be classified from their log output.'
  - `AppMsg::ConnectionHealth or AppMsg::DnsHealth carrying a superseded generation` → `app_health_none` → **no-op** — anchor '                if !is_current_generation(generation, self.connection_generation) {' in the AppMsg::ProcessLogLine arm — the same guard
  - `App::apply_state with any state other than ProcessState::Running` → `app_health_none` → **clear** — anchor '        self.process_state = state.clone();' — health and dns_failing are cleared here so a stale Unhealthy can never colour a Starting
  - `status_texts with Running, metadata present, health Unhealthy(reason)` → `app_health_unhealthy` → **set** — requirement: 'An unhealthy session SHALL show `Proxy not responding` as the status text, with the connection details followed by the last probe failure.'
  - `status_texts with Running, metadata present, health Healthy, dns_failing true` → `app_dns_failing` → **set** — requirement: 'A healthy session whose DNS through the proxy is marked failing SHALL show `Connected` with the details prefixed by `DNS via proxy failing`.'
  - `status_texts with Running, metadata present, health Healthy, dns_failing false` → `app_health_healthy` → **no-op** — requirement: 'A healthy session without the DNS mark SHALL show the existing connected text.' — anchor '                ("Connected".to_string(), details)'
  - `status_texts with Starting while a stale Unhealthy is still in the view struct` → `app_health_none` → **no-op** — sprint arm table: health is read only in the Running arms — anchor '            (ProcessState::Starting, _) => ("Connecting…".to_string(), "Resolving nodes".into()),'
  - `Unhealthy transition while health_check.failover is true, session_target is None and health_failovers < MAX_AUTO_RECONNECTS` → `failover_armed` → **set** — requirement: 'When it is on and a session started by the configured strategy becomes unhealthy, the system SHALL disconnect and reconnect using the configured strategy with the unhealthy node excluded from that attempt's candidates.'
  - `Unhealthy transition while health_check.failover is false` → `failover_blocked` → **no-op** — requirement scenario 'Failover off by default'
  - `Unhealthy transition while session_target is Some(_)` → `failover_blocked` → **no-op** — requirement: 'A session started by a direct connection to a chosen node SHALL NOT fail over for health, regardless of the preference.'
  - `Unhealthy transition while health_failovers == MAX_AUTO_RECONNECTS` → `failover_blocked` → **no-op** — requirement: 'The system SHALL perform at most 3 consecutive health failovers without a successful probe in between' — anchor 'const MAX_AUTO_RECONNECTS: u32 = 3;'
  - `terminal Stopped arriving while health_reconnect_pending is true` → `failover_armed` → **forced** — requirement: 'the system SHALL disconnect and reconnect using the configured strategy' — dispatches AppMsg::Connect(ConnectOrigin::AutoReconnect) directly, not through the reconnect_pending path at anchor '                    sender.input(reconnect_msg(self.session_target, ConnectOrigin::Restart));' which would force ConnectOrigin::Restart
  - `AppMsg::Connect handler after planner.plan` → `failover_armed` → **clear** — anchor '                let candidates = planner.plan(&subscriptions, &manual_nodes);' — self.excluded_node.take() removes the node for this attempt only
  - `AppMsg::Connect or AppMsg::ConnectToNode with cancels_auto_reconnect(origin) true, or AppMsg::Disconnect` → `failover_blocked` → **clear** — requirement: 'a user-initiated Connect, direct connect, or Disconnect SHALL reset that count' — anchor 'fn cancels_auto_reconnect(origin: ConnectOrigin) -> bool {'
  - `AppMsg::ConnectionHealth carrying Health::Healthy for the current generation` → `failover_blocked` → **clear** — requirement: 'a successful probe ... SHALL reset that count' — health_failovers = 0
  - `AppMsg::Disconnect issued by the health failover (health_reconnect_pending true)` → `failover_armed` → **no-op** — crates/ui/src/app.rs Disconnect arm calls cancel_auto_reconnect() on every Disconnect; the self-issued stop must keep excluded_node and health_failovers
  - `recheck tick with no counted line in the last 60 s` → `dns_clear` → **set** — emits AppMsg::DnsHealth(generation, false); DnsFailureWindow::is_failing(now) is false once DNS_WINDOW has elapsed since the newest counted line
- forbidden:
  - resetting health_failovers inside cancel_auto_reconnect (anchor '    fn cancel_auto_reconnect(&mut self) {') — it runs on every ProcessState::Running at anchor '                            self.cancel_auto_reconnect();', which would refill the budget after each failover and allow an unbounded loop
  - reusing auto_reconnect_attempts (anchor '    auto_reconnect_attempts: u32,') as the health failover counter, for the same reason
  - routing the health failover through self.reconnect_pending — the path at anchor '                    sender.input(reconnect_msg(self.session_target, ConnectOrigin::Restart));' hardcodes ConnectOrigin::Restart, and cancels_auto_reconnect(Restart) is true, so the Connect arm would clear excluded_node and reset health_failovers before the plan is filtered
  - reading self.connection_status for the excluded node after dispatching AppMsg::Disconnect — the node_ref must be captured in the ConnectionHealth handler first
  - spawning the monitor with the same broadcast receiver the state_forwarder holds — it takes a second mgr.subscribe(); the forwarder at anchor '            let mut state_rx = mgr.subscribe();' stays untouched
  - returning from any post-Running exit path of the supervision loop without halting the monitor — a surviving monitor keeps connecting to the proxy port after handle.stop(), which this seam's own test asserts against
  - probing at all when settings.health_check.enabled is false
  - adding health_check to RuntimeConfigSnapshot
  - hardcoding 10 s / 30 s inside the monitor body with no test-visible override — the harness cannot wait 10 s inside RECV_TIMEOUT = 20 s for a three-failure streak
- seeding:
  - ProcessState::Running for a connection: only `connect(&stub, settings, vec![candidate(addr)])` (or the new connect_with_health) followed by `next_state(&rx)`. Never by constructing a ProcessState or emitting AppMsg by hand.
  - monitor_paused: only a stub script that exits on its own so ProcessManager::handle_unexpected_exit respawns it (the existing crash-respawn stub pattern in crates/ui/src/connection.rs tests). Never by publishing a StateChanged event directly.
  - monitor_halted: only `handle.stop()` followed by `assert_nothing_after_terminal(&rx)` plus an assertion that the fake proxy listener accepted no further connection.
  - the probe endpoint: bind a tokio::net::TcpListener on 127.0.0.1:0, read its port, and pass that port as `http` to `v2ray_settings(socks, http, "127.0.0.1", false)`; the listener counts accepts and switches its canned reply from 204 to 502 on command.
  - app_health_* and failover_* states: assert on the pure helpers only — `status_texts`, `should_announce`, `health_failover_allowed`, `exclude_candidate`. crates/ui/src/app.rs has no headless harness for App::update (its tests build fixtures like `snapshot(backend, tun_enabled)` and `session_target_node()` and call free functions), so no test constructs an App or drives a message through it.
  - the xray DNS path: a stub script that prints 25 `app/dns: failed to retrieve response` lines, run with a settings value whose backend_type is Xray, drained with `drain(&rx)`; the sing-box counterpart uses `singbox_settings()` with the same script.
- budgets:
  - HEALTH_INITIAL_DELAY = Duration::from_secs(10)
  - HEALTH_INTERVAL = Duration::from_secs(30)
  - per-probe timeout = Duration::from_millis(u64::from(settings.real_delay.timeout_ms)) — the field is u32 milliseconds
  - FAILURE_THRESHOLD = 3 consecutive failures before AppMsg::ConnectionHealth(_, Health::Unhealthy(_))
  - MAX_AUTO_RECONNECTS = 3 health failovers
  - test timing override: HealthTiming { initial_delay: Duration::from_millis(50), interval: Duration::from_millis(100) } so a three-failure streak lands well inside RECV_TIMEOUT = 20 s
  - existing connection tests keep HealthTiming::default(), whose 10 s first probe never fires inside their lifetime
  - DNS_RECHECK_INTERVAL: Duration::from_secs(10) — the timer tick that re-evaluates DnsFailureWindow::is_failing so the 60 s clear edge fires without a further log line

**`tray-notify`** — tasks 4.2
- states: `announced`, `quiet`
- transitions:
  - `should_announce(None, &Health::Unhealthy(reason))` → `announced` → **set** — requirement: 'When a session becomes unhealthy, the system SHALL show a notification in the main window'
  - `should_announce(Some(&Health::Healthy), &Health::Unhealthy(reason))` → `announced` → **set** — requirement scenario 'First transition notifies'
  - `should_announce(Some(&Health::Unhealthy(_)), &Health::Unhealthy(_))` → `quiet` → **no-op** — requirement: 'The system SHALL NOT repeat either while the session stays unhealthy.'
  - `should_announce(_, &Health::Healthy)` → `quiet` → **no-op** — requirement names only the unhealthy transition as announced
  - `an announced transition while settings.notifications_enabled is false` → `announced` → **set** — requirement: 'and, when desktop notifications are enabled, SHALL send a desktop notification' — the toast still shows, the desktop notification does not
- forbidden:
  - calling Notification::show on the GTK main thread — anchor '    fn send(&self, summary: &str, body: &str) {' is blocking; the tray service wraps its calls in tokio::task::spawn_blocking and the new path must do the same
  - dropping the enabled gate — Notifier owns it (anchor '        if !self.enabled.load(Ordering::Relaxed) {' in on_state_change); the new public wrapper keeps the check inside the Notifier rather than at the call site
  - holding the TRAY_HANDLE mutex across the blocking show call
  - announcing from anywhere but the ConnectionHealth handler's transition edge
- seeding:
  - announced / quiet: only by calling the pure `should_announce(prev, next)` with Health values built directly. No test drives a real notification: the desktop side needs a live org.freedesktop.Notifications service and is covered by the manual verification step.
- budgets:
  - at most one toast and one desktop notification per Healthy-or-None -> Unhealthy edge
  - NOTIFICATION_TIMEOUT_MS = 5000, unchanged
  - one new unit test in this seam

**`verification`** — tasks 5.1, 5.2
- states: `workspace_green`, `live_unhealthy_observed`, `live_recovered`
- transitions:
  - `TEST_TIMEOUT=10m make test` → `workspace_green` → **set** — Makefile anchor 'TEST := timeout $(TEST_TIMEOUT) $(CARGO) test' with 'TEST_ARGS := -- --test-threads=$(TEST_THREADS)'
  - `live: nft rule dropping the connected node's address while a session runs` → `live_unhealthy_observed` → **set** — task 5.2 — status shows `Proxy not responding` with exactly one toast within about 2 minutes
  - `live: the nft rule removed` → `live_recovered` → **forced** — task 5.2 — status returns to `Connected` with no toast
- forbidden:
  - a bare `cargo test` without the timeout and --test-threads cap — the Makefile targets carry both
  - treating 5.2's absence as a failure of the change; it is marked manual
- seeding:
  - workspace_green: only the Makefile targets below.
  - live_unhealthy_observed / live_recovered: only a real xray backend with a real subscription and a root-installed nft rule in a dedicated test table, removed afterwards. Not reachable from any test harness.
- budgets:
  - workspace test wall clock: 10 minutes
  - test threads: 4
  - live detection window: about 2 minutes (10 s initial delay + 3 intervals of 30 s + probe timeouts)
  - expected toasts during the live outage: exactly 1

### Floor

TEST_TIMEOUT=10m make test && make lint   — `make test` expands to `timeout 10m cargo test --workspace --all-targets -- --test-threads=4` (Makefile anchors 'TEST := timeout $(TEST_TIMEOUT) $(CARGO) test' and 'TEST_ARGS := -- --test-threads=$(TEST_THREADS)'); `make lint` runs `cargo fmt -- --check` then `cargo clippy --workspace --all-targets --all-features -- -D warnings`. Plus the manual live step of task 5.2, which needs a real xray binary, a real subscription and root nft rules and is not runnable in a harness.


### Requirements map

- ### Requirement: Opt-in failover of an unhealthy session The system SHALL provide a preference, off by default, to fail over a session that becomes unhealthy. When it is on and a session started by th…
  - tests: `crates/subscription/src/health.rs::tests::probe_succeeds_on_204`; `crates/subscription/src/health.rs::tests::probe_fails_on_502`; `crates/subscription/src/health.rs::tests::probe_fails_on_timeout`
- - **THEN** the session SHALL keep running on the same node
  - tests: `MANUAL task 5.2: live xray check — block the node address, observe the status text, the single toast, the recovery, and with failover on the node the reconnect picked`
- - **THEN** the system SHALL reconnect with the configured strategy and node A SHALL NOT be a candidate for that attempt
  - tests: `crates/ui/src/app.rs::tests::health_failover_allowed_rejects_direct_target`; `crates/ui/src/app.rs::tests::exclude_candidate_drops_only_the_named_node`; `crates/ui/src/app.rs::tests::health_failover_allowed_rejects_exhausted_budget`; `crates/ui/src/app.rs::tests::health_failover_allowed_rejects_disabled_preference`
- - **THEN** the session SHALL keep running on node A and only the unhealthy status SHALL be shown
  - tests: `crates/ui/src/health.rs::tests::two_failures_report_nothing_third_reports_unhealthy`; `crates/ui/src/health.rs::tests::further_failures_stay_quiet`
- - **THEN** the system SHALL NOT fail over again and SHALL keep the last session running with the unhealthy status
  - tests: `crates/ui/src/health.rs::tests::two_failures_report_nothing_third_reports_unhealthy`; `crates/ui/src/health.rs::tests::further_failures_stay_quiet`
- ### Requirement: Running connection is probed through its own inbound While a connection is `Running` and health checks are enabled (the default), the system SHALL request the configured Real Delay te…
  - tests: `crates/subscription/src/health.rs::tests::probe_succeeds_on_204`; `crates/subscription/src/health.rs::tests::probe_fails_on_502`; `crates/subscription/src/health.rs::tests::probe_fails_on_timeout`
- - **THEN** the session SHALL be unhealthy while its process state remains `Running`
  - tests: `crates/ui/src/health.rs::tests::two_failures_report_nothing_third_reports_unhealthy`; `crates/ui/src/health.rs::tests::further_failures_stay_quiet`
- - **THEN** the session SHALL be healthy
  - tests: `MANUAL task 5.2: live xray check — block the node address, observe the status text, the single toast, the recovery, and with failover on the node the reconnect picked`
- - **THEN** the session SHALL stay healthy
  - tests: `MANUAL task 5.2: live xray check — block the node address, observe the status text, the single toast, the recovery, and with failover on the node the reconnect picked`
- - **THEN** no probe SHALL be sent until the respawned backend is `Running`, and the session SHALL then be healthy until three new consecutive failures
  - tests: `crates/subscription/src/health.rs::tests::probe_succeeds_on_204`; `crates/subscription/src/health.rs::tests::probe_fails_on_502`; `crates/subscription/src/health.rs::tests::probe_fails_on_timeout`
- - **THEN** no probe SHALL be sent and the session SHALL never be marked unhealthy
  - tests: `crates/subscription/src/health.rs::tests::probe_succeeds_on_204`; `crates/subscription/src/health.rs::tests::probe_fails_on_502`; `crates/subscription/src/health.rs::tests::probe_fails_on_timeout`
- ### Requirement: Unhealthy session is announced once per streak When a session becomes unhealthy, the system SHALL show a notification in the main window stating that the proxy is not responding and n…
  - tests: `crates/subscription/src/health.rs::tests::probe_succeeds_on_204`; `crates/subscription/src/health.rs::tests::probe_fails_on_502`; `crates/subscription/src/health.rs::tests::probe_fails_on_timeout`
- - **THEN** one in-window notification and one desktop notification SHALL be shown
  - tests: `crates/ui/src/app.rs::tests::should_announce_fires_once_per_unhealthy_streak`
- - **THEN** no new notification SHALL be shown
  - tests: `crates/ui/src/app.rs::tests::should_announce_fires_once_per_unhealthy_streak`
- ### Requirement: xray DNS through the proxy is flagged when failing When the backend is xray, the system SHALL count backend log lines reporting `app/dns: failed to retrieve response` and SHALL mark D…
  - tests: `crates/ui/src/health.rs::tests::two_failures_report_nothing_third_reports_unhealthy`; `crates/ui/src/health.rs::tests::further_failures_stay_quiet`
- - **THEN** DNS through the proxy SHALL be marked failing
  - tests: `crates/ui/src/health.rs::tests::dns_window_marks_failing_at_twenty_within_sixty_seconds`; `crates/ui/src/app.rs::tests::status_texts_dns_failing_prefixes_details`; `crates/ui/src/health.rs::tests::dns_window_clears_sixty_seconds_after_the_last_line`; `crates/ui/src/health.rs::tests::dns_window_ignores_tun_noise`
- - **THEN** the session SHALL stay healthy and DNS SHALL NOT be marked failing
  - tests: `crates/ui/src/health.rs::tests::dns_window_marks_failing_at_twenty_within_sixty_seconds`; `crates/ui/src/app.rs::tests::status_texts_dns_failing_prefixes_details`; `crates/ui/src/health.rs::tests::dns_window_clears_sixty_seconds_after_the_last_line`; `crates/ui/src/health.rs::tests::dns_window_ignores_tun_noise`
- - **THEN** the mark SHALL clear
  - tests: `MANUAL task 5.2: live xray check — block the node address, observe the status text, the single toast, the recovery, and with failover on the node the reconnect picked`
- ### Requirement: Status bar shows connection health While the connection is `Running`, the status bar SHALL reflect connection health. An unhealthy session SHALL show `Proxy not responding` as the sta…
  - tests: `crates/subscription/src/health.rs::tests::probe_succeeds_on_204`; `crates/subscription/src/health.rs::tests::probe_fails_on_502`; `crates/subscription/src/health.rs::tests::probe_fails_on_timeout`
- - **THEN** the status text SHALL be `Proxy not responding` and the details SHALL end with the failure reason
  - tests: `crates/ui/src/app.rs::tests::status_texts_unhealthy_says_proxy_not_responding`; `crates/ui/src/app.rs::tests::status_texts_healthy_matches_todays_connected_text`; `crates/ui/src/app.rs::tests::status_texts_starting_ignores_stale_health`
- - **THEN** the status text SHALL be `Connected` and the details SHALL start with `DNS via proxy failing`
  - tests: `crates/ui/src/health.rs::tests::dns_window_marks_failing_at_twenty_within_sixty_seconds`; `crates/ui/src/app.rs::tests::status_texts_dns_failing_prefixes_details`; `crates/ui/src/health.rs::tests::dns_window_clears_sixty_seconds_after_the_last_line`; `crates/ui/src/health.rs::tests::dns_window_ignores_tun_noise`
- - **THEN** the status bar SHALL show the same text as any connected session
  - tests: `crates/ui/src/app.rs::tests::status_texts_unhealthy_says_proxy_not_responding`; `crates/ui/src/app.rs::tests::status_texts_healthy_matches_todays_connected_text`; `crates/ui/src/app.rs::tests::status_texts_starting_ignores_stale_health`

### Plan review

`pass` by zarchitect, 2 rounds.

- Round 1 raised 6 blockers, all fixed: h4 added two AppMsg variants whose handler arms were h5's work against an exhaustive match (E0004 at h4's own verify); h4 compiled h1's symbols with no edge to h1; the failover dispatched AppMsg::Disconnect whose arm cleared its own exclusion and budget, so the dead node was re-picked without bound; status_texts had two contradictory signatures and StatusView existed nowhere; the xray DNS mark could never clear because is_failing was only evaluated inside the LogLine arm; and three requirement-map entries named tests no codeTask creates.
- Round 2 raised 2 more, both fixed: the seams[] mirror was stale — all three round-2 fixes existed only in chunks[], so any stage reading seams[].codeTasks would have re-introduced every defect verbatim (seams are now regenerated from chunks, and the same parity was checked and repaired across all four plans of the sprint); and the DNS recheck timer had no number, no transition row and no test (DNS_RECHECK_INTERVAL = 10 s is now a budget, the timer-driven clear has a transition row, and dns_window_clears_after_the_recheck_tick asserts it).
- Cross-change: this change lands last and depends on AppSettings::local_endpoint (confirm-backend-ready-before-running) and on StatusView (log-connection-decisions). Both dependencies are stated in the plan with a fallback if absent.
- Accepted: h2 and h3 are parallel with sharedPkg null while both build v2ray-rs-ui — the files they write do not overlap (h3 is confined to crates/ui/src/preferences/network.rs and is forbidden from touching app.rs), so this is a convention mismatch, not a conflict.
- Accepted, carried from round 1: negative assertions ('zero accepts', 'no DnsHealth') carry no settle window, and the probe can be routed Direct for the default test_url.
- Task 5.2 stays MANUAL: it needs a real xray, a real subscription and root nft rules.

## Plan appendix

```json
{
  "v": 2,
  "change": "detect-dead-proxy-link",
  "baseSha": "1d6a0fb8eb02e73a65fc393adfdd743e2bba742f",
  "generatedAt": "2026-09-16T09:43:45.431386+00:00",
  "tier": "heavy",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality",
    "arch"
  ],
  "chunks": [
    {
      "id": "h1",
      "taskIds": [
        "1.1",
        "1.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "settings-and-probe",
      "shard": "core-sub",
      "pkgDirs": [
        "crates/core/src/models",
        "crates/subscription/src"
      ],
      "pkgs": [
        "v2ray-rs-core",
        "v2ray-rs-subscription"
      ],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/models/settings.rs",
          "symbol": "HealthCheckSettings (new struct)",
          "anchor": "impl Default for RealDelaySettings {",
          "change": "new `HealthCheckSettings { enabled: bool, failover: bool }` with Debug/Clone/PartialEq/Eq/Serialize/Deserialize and a manual Default (enabled true, failover false), placed next to RealDelaySettings"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/models/settings.rs",
          "symbol": "AppSettings",
          "anchor": "    pub real_delay: RealDelaySettings,",
          "change": "add `#[serde(default)] pub health_check: HealthCheckSettings,` after this field"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/models/settings.rs",
          "symbol": "impl Default for AppSettings",
          "anchor": "            real_delay: RealDelaySettings::default(),",
          "change": "add `health_check: HealthCheckSettings::default(),`"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/models/settings.rs",
          "symbol": "mod tests",
          "anchor": "    fn test_legacy_settings_toml_missing_real_delay_defaults() {",
          "change": "clone this test shape for health_check: legacy TOML without the table loads enabled=true/failover=false, plus a TOML round-trip test like test_real_delay_settings_round_trip"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/models/mod.rs",
          "symbol": "pub use settings::*",
          "anchor": "pub use settings::{",
          "change": "add HealthCheckSettings to this explicit re-export list"
        },
        {
          "task": "1.2",
          "file": "crates/subscription/src/health.rs",
          "symbol": "probe_via_http_proxy (new file)",
          "anchor": "",
          "change": "NEW FILE: `pub async fn probe_via_http_proxy(proxy: SocketAddr, url: &str, timeout: Duration) -> Result<(), String>`; reqwest::Client::builder().proxy(reqwest::Proxy::all(format!(\"http://{proxy}\"))).timeout(timeout).build(); status < 400 -> Ok, else Err(status text); transport error -> Err(e.to_string()) (reqwest timeout errors stringify containing `operation timed out`)"
        },
        {
          "task": "1.2",
          "file": "crates/subscription/src/lib.rs",
          "symbol": "module list",
          "anchor": "pub(crate) mod real_delay;",
          "change": "add `pub(crate) mod health;` in alphabetical position (after fetch)"
        },
        {
          "task": "1.2",
          "file": "crates/subscription/src/lib.rs",
          "symbol": "re-exports",
          "anchor": "pub use real_delay::{RealDelayReport, measure_real_delay};",
          "change": "add `pub use health::probe_via_http_proxy;` — required, the ui crate calls it and every module here is pub(crate)"
        }
      ],
      "contract": {
        "states": [
          "health_check_defaults",
          "health_check_custom",
          "probe_ok",
          "probe_status_error",
          "probe_timeout",
          "probe_transport_error"
        ],
        "transitions": [
          {
            "input": "settings TOML carrying no [health_check] table",
            "state": "health_check_defaults",
            "effect": "set",
            "evidence": "anchor 'fn test_legacy_settings_toml_missing_real_delay_defaults() {' — the same shape, cloned for health_check"
          },
          {
            "input": "AppSettings::default()",
            "state": "health_check_defaults",
            "effect": "set",
            "evidence": "anchor '            real_delay: RealDelaySettings::default(),' in impl Default for AppSettings"
          },
          {
            "input": "toml::to_string then toml::from_str of an AppSettings with health_check.failover = true",
            "state": "health_check_custom",
            "effect": "no-op",
            "evidence": "anchor 'fn test_real_delay_settings_round_trip() {' — asserts full AppSettings equality after the round trip"
          },
          {
            "input": "proxy listener answers 'HTTP/1.1 204 No Content'",
            "state": "probe_ok",
            "effect": "set",
            "evidence": "requirement: 'A response with a status below 400 SHALL count as a success'"
          },
          {
            "input": "proxy listener answers 'HTTP/1.1 502 Bad Gateway'",
            "state": "probe_status_error",
            "effect": "set",
            "evidence": "requirement: 'A response with a status below 400 SHALL count as a success; a connection, TLS, or protocol error or a timeout SHALL count as a failure'"
          },
          {
            "input": "proxy listener accepts and never writes, probe timeout 300 ms",
            "state": "probe_timeout",
            "effect": "set",
            "evidence": "requirement: 'or a timeout SHALL count as a failure'"
          },
          {
            "input": "no listener bound at the proxy address",
            "state": "probe_transport_error",
            "effect": "set",
            "evidence": "requirement: 'a connection, TLS, or protocol error ... SHALL count as a failure'"
          }
        ],
        "forbidden": [
          "health_check added to RuntimeConfigSnapshot — verified absent from the literal at anchor 'self.runtime_snapshot = Some(RuntimeConfigSnapshot {'; adding it would make check_restart_required raise the restart banner on a toggle, contradicting task 4.4",
          "health_check declared before any scalar field of AppSettings — toml::to_string fails with ValueAfterTable; it must sit after 'pub real_delay: RealDelaySettings,', which is today the last field",
          "a socks:// proxy URL — the workspace reqwest pin 'reqwest = { version = \"0.13\", features = [\"rustls-no-provider\", \"stream\"], default-features = false }' has no `socks` feature; only http:// proxies are reachable",
          "classifying a timeout by matching reqwest::Error's Display text — the probe must branch on err.is_timeout() and emit its own message containing 'timed out'",
          "probe_via_http_proxy left crate-private — every module in crates/subscription/src/lib.rs is pub(crate), so without an explicit re-export the ui crate cannot call it"
        ],
        "seeding": [
          "health_check_defaults: only via toml::from_str on a TOML string with no [health_check] table, or AppSettings::default(). Never by writing the field on a constructed struct.",
          "health_check_custom: only via an AppSettings struct literal with '..AppSettings::default()', mirroring 'fn test_real_delay_settings_round_trip'.",
          "probe_ok / probe_status_error / probe_timeout: only by calling probe_via_http_proxy against a tokio::net::TcpListener bound to 127.0.0.1:0 that speaks raw HTTP/1.1, modelled on 'async fn spawn_mock_clash(responses: Vec<Option<u64>>) -> u16' in crates/subscription/src/real_delay.rs. The probe URL in these tests MUST use the http:// scheme: with an https:// URL reqwest sends CONNECT to the proxy, which the raw listener does not implement.",
          "probe_transport_error: bind a listener with pick_free_loopback_port()'s pattern, drop it, then probe that port."
        ],
        "budgets": [
          "probe timeout in the timeout test: 300 ms",
          "success status threshold: status < 400",
          "health_check defaults: enabled = true, failover = false",
          "three probe tests total (204, 502, no-answer); a fourth for the unbound port is optional"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST: in crates/core/src/models/settings.rs `mod tests`, add `test_legacy_settings_toml_missing_health_check_defaults` (TOML string identical to the one at anchor 'fn test_legacy_settings_toml_missing_real_delay_defaults() {', asserting settings.health_check.enabled == true and settings.health_check.failover == false) and `test_health_check_settings_round_trip` (AppSettings with `health_check: HealthCheckSettings { enabled: false, failover: true }`, `..AppSettings::default()`, toml::to_string -> from_str, assert_eq on the whole AppSettings).",
        "TEST FIRST: create crates/subscription/src/health.rs with `mod tests` holding `probe_succeeds_on_204`, `probe_fails_on_502`, `probe_fails_on_timeout` (asserting the Err string contains `timed out`), each using a local tokio::net::TcpListener and an http:// URL.",
        "Add `HealthCheckSettings { pub enabled: bool, pub failover: bool }` to crates/core/src/models/settings.rs next to RealDelaySettings (anchor 'impl Default for RealDelaySettings {'), deriving Debug, Clone, PartialEq, Eq, Serialize, Deserialize, with a manual `impl Default` setting enabled = true, failover = false.",
        "Add `#[serde(default)] pub health_check: HealthCheckSettings,` immediately after the anchor '    pub real_delay: RealDelaySettings,' in AppSettings, and `health_check: HealthCheckSettings::default(),` after the anchor '            real_delay: RealDelaySettings::default(),' in impl Default.",
        "Add `HealthCheckSettings` to the explicit re-export list at anchor 'pub use settings::{' in crates/core/src/models/mod.rs.",
        "Implement `pub async fn probe_via_http_proxy(proxy: SocketAddr, url: &str, timeout: Duration) -> Result<(), String>` in crates/subscription/src/health.rs: build a per-call client with `reqwest::Client::builder().proxy(reqwest::Proxy::all(format!(\"http://{proxy}\")).map_err(|e| e.to_string())?).timeout(timeout).build()`, GET the url, return Ok(()) when `response.status().as_u16() < 400`, otherwise `Err(status.to_string())`; on a transport error return `Err(\"timed out\".to_string())` when `err.is_timeout()` and `Err(err.to_string())` otherwise.",
        "Register the module: add `pub(crate) mod health;` after the anchor 'pub(crate) mod fetch;' in crates/subscription/src/lib.rs, and `pub use health::probe_via_http_proxy;` beside the anchor 'pub use real_delay::{RealDelayReport, measure_real_delay};'."
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-core && make test-subscription && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "h2",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "prev": "h1",
      "sharedPkg": null,
      "parallel": true,
      "seam": "ui-health-trackers",
      "shard": "ui-pure",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/ui/src/health.rs",
          "symbol": "HealthTracker, Health (new file)",
          "anchor": "",
          "change": "NEW FILE (recommended over growing connection.rs): pure `Health` enum {Healthy, Unhealthy(String)}, `HealthTracker::record(Result<(), String>) -> Option<Health>` returning a transition only, `reset()`; FAILURE_THRESHOLD = 3 const here"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/lib.rs",
          "symbol": "module list",
          "anchor": "pub(crate) mod failure_streak;",
          "change": "add `pub(crate) mod health;` after this line"
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/health.rs",
          "symbol": "DnsFailureWindow",
          "anchor": "",
          "change": "same new file: `observe(&mut self, line: &str, now: Instant)` counts only lines containing `app/dns: failed to retrieve response`; `is_failing(&self, now: Instant) -> bool` true at >= 20 within 60 s, clears 60 s after the last counted line; constants DNS_FAILURE_THRESHOLD/DNS_WINDOW local"
        }
      ],
      "contract": {
        "states": [
          "healthy",
          "failing_1",
          "failing_2",
          "unhealthy",
          "dns_clear",
          "dns_failing"
        ],
        "transitions": [
          {
            "input": "record(Err(reason)) with 0 consecutive failures recorded",
            "state": "failing_1",
            "effect": "no-op",
            "evidence": "requirement: 'After 3 consecutive failures the session SHALL be unhealthy'"
          },
          {
            "input": "record(Err(reason)) with 1 consecutive failure recorded",
            "state": "failing_2",
            "effect": "no-op",
            "evidence": "requirement: 'After 3 consecutive failures the session SHALL be unhealthy'"
          },
          {
            "input": "record(Err(reason)) with 2 consecutive failures recorded",
            "state": "unhealthy",
            "effect": "set",
            "evidence": "requirement: 'After 3 consecutive failures the session SHALL be unhealthy' — returns Some(Health::Unhealthy(reason))"
          },
          {
            "input": "record(Err(reason)) while already unhealthy",
            "state": "unhealthy",
            "effect": "no-op",
            "evidence": "requirement scenario 'Continued failures stay quiet'"
          },
          {
            "input": "record(Ok(())) while unhealthy",
            "state": "healthy",
            "effect": "set",
            "evidence": "requirement: 'the next success SHALL make it healthy' — returns Some(Health::Healthy)"
          },
          {
            "input": "record(Ok(())) while last_reported is None (fresh tracker or after reset)",
            "state": "healthy",
            "effect": "set",
            "evidence": "connection-auto-resolve requirement: 'a successful probe ... SHALL reset that count' — without this edge the failover budget can never be reset by a probe; returns Some(Health::Healthy)"
          },
          {
            "input": "record(Ok(())) while last_reported is Some(Health::Healthy)",
            "state": "healthy",
            "effect": "no-op",
            "evidence": "requirement scenario 'Isolated failure': one failure then a success leaves the session healthy and reports nothing new"
          },
          {
            "input": "record(Err(_)) after a single earlier failure followed by record(Ok(()))",
            "state": "failing_1",
            "effect": "forced",
            "evidence": "requirement: 'After 3 consecutive failures' — the counter is reset by any success"
          },
          {
            "input": "reset()",
            "state": "healthy",
            "effect": "forced",
            "evidence": "requirement: 'Probing SHALL pause while the backend is being respawned and restart from a healthy state' — clears the counter and last_reported, returns nothing"
          },
          {
            "input": "observe(line containing 'app/dns: failed to retrieve response', now)",
            "state": "dns_clear",
            "effect": "set",
            "evidence": "requirement: 'the system SHALL count backend log lines reporting `app/dns: failed to retrieve response`' — records the timestamp"
          },
          {
            "input": "observe(line containing 'proxy/tun: connection reset by peer', now)",
            "state": "dns_clear",
            "effect": "no-op",
            "evidence": "requirement: 'Log lines that carry no destination, such as `proxy/tun: connection reset by peer` and `proxy/tun: connection was refused`, SHALL NOT affect health'"
          },
          {
            "input": "is_failing(now) with 20 or more counted lines whose timestamps are within 60 s of now",
            "state": "dns_failing",
            "effect": "set",
            "evidence": "requirement: 'SHALL mark DNS through the proxy as failing when at least 20 such lines occur within 60 seconds'"
          },
          {
            "input": "is_failing(now) 60 s after the newest counted line",
            "state": "dns_clear",
            "effect": "forced",
            "evidence": "requirement: 'the mark SHALL clear after 60 seconds without such a line'"
          }
        ],
        "forbidden": [
          "reset() returning a transition — the App has already cleared health on the non-Running state that caused the reset, so a Healthy emission there would repaint a Starting status bar",
          "DnsFailureWindow reading Instant::now() internally — `now` is a parameter on both observe and is_failing, so tests drive synthetic time",
          "DnsFailureWindow counting any line whose text starts with 'proxy/tun:'",
          "Health without a Debug derive — AppMsg is declared '#[derive(Debug)]' at anchor 'pub enum AppMsg {' and will not compile otherwise",
          "HealthTracker holding a reqwest client, a SocketAddr or any IO — it records outcomes only"
        ],
        "seeding": [
          "healthy / failing_1 / failing_2 / unhealthy: only `HealthTracker::new()` followed by `record(Ok(()))` / `record(Err(String))` calls. Never by writing the internal counter.",
          "the after-reset healthy state: only `HealthTracker::reset()`.",
          "dns_clear / dns_failing: only `DnsFailureWindow::default()` then `observe(line, now)` with Instants built as `base + Duration::from_secs(k)` from one `Instant::now()` captured at the top of the test."
        ],
        "budgets": [
          "FAILURE_THRESHOLD = 3 (consecutive probe failures before Unhealthy)",
          "DNS_FAILURE_THRESHOLD = 20 (counted lines)",
          "DNS_WINDOW = Duration::from_secs(60)",
          "noise test feeds 500 'proxy/tun: connection reset by peer' lines and asserts is_failing stays false",
          "DNS_RECHECK_INTERVAL: Duration::from_secs(10) — the timer tick that re-evaluates DnsFailureWindow::is_failing so the 60 s clear edge fires without a further log line"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST: create the `mod tests` in crates/ui/src/health.rs with `two_failures_report_nothing_third_reports_unhealthy`, `further_failures_stay_quiet`, `success_after_unhealthy_reports_healthy`, `first_success_of_a_session_reports_healthy`, `success_between_failures_resets_the_streak`, `reset_clears_the_streak_and_reports_nothing`, `dns_window_marks_failing_at_twenty_within_sixty_seconds`, `dns_window_clears_sixty_seconds_after_the_last_line`, `dns_window_ignores_tun_noise` (500 lines).",
        "Create crates/ui/src/health.rs with `#[derive(Debug, Clone, PartialEq, Eq)] pub(crate) enum Health { Healthy, Unhealthy(String) }`, `const FAILURE_THRESHOLD: u32 = 3;`, and `pub(crate) struct HealthTracker { failures: u32, last_reported: Option<Health> }` exposing `new()`, `record(&mut self, outcome: Result<(), String>) -> Option<Health>` and `reset(&mut self)` per the transition table.",
        "Add `pub(crate) struct DnsFailureWindow` to the same file: `observe(&mut self, line: &str, now: Instant)` pushes `now` only when `line.contains(\"app/dns: failed to retrieve response\")` and drops entries older than DNS_WINDOW; `is_failing(&self, now: Instant) -> bool` counts entries within DNS_WINDOW of `now` and compares against DNS_FAILURE_THRESHOLD. Constants `DNS_FAILURE_THRESHOLD: usize = 20` and `DNS_WINDOW: Duration = Duration::from_secs(60)` live in this file.",
        "Register the module: add `pub(crate) mod health;` after the anchor 'pub(crate) mod failure_streak;' in crates/ui/src/lib.rs."
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-ui && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "h3",
      "taskIds": [
        "4.4"
      ],
      "prev": "h1",
      "sharedPkg": null,
      "parallel": true,
      "seam": "preferences-page",
      "shard": "ui-prefs",
      "pkgDirs": [
        "crates/ui/src/preferences"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "4.4",
          "file": "crates/ui/src/preferences/network.rs",
          "symbol": "build_network_page (rows)",
          "anchor": "    let real_delay_use_for_lowest_row = adw::SwitchRow::builder()",
          "change": "add a `Connection health` PreferencesGroup after real_delay_group with two adw::SwitchRow rows: \"Check connection health\" (health_check.enabled) and \"Reconnect when the proxy stops responding\" (health_check.failover); build them before `drop(s);`"
        },
        {
          "task": "4.4",
          "file": "crates/ui/src/preferences/network.rs",
          "symbol": "build_network_page (handlers)",
          "anchor": "        real_delay_use_for_lowest_row.connect_active_notify(move |row| {",
          "change": "copy this block twice: `st.borrow_mut().health_check.enabled/.failover = row.is_active(); emit(&st, &cb);`"
        }
      ],
      "contract": {
        "states": [
          "row_reflects_settings",
          "settings_updated",
          "restart_banner_quiet"
        ],
        "transitions": [
          {
            "input": "page built with health_check.enabled = true",
            "state": "row_reflects_settings",
            "effect": "set",
            "evidence": "anchor '    let real_delay_use_for_lowest_row = adw::SwitchRow::builder()' — .active(s.real_delay.use_for_lowest_latency) is the exact binding shape"
          },
          {
            "input": "user toggles 'Check connection health'",
            "state": "settings_updated",
            "effect": "set",
            "evidence": "anchor '        real_delay_use_for_lowest_row.connect_active_notify(move |row| {' — st.borrow_mut() mutation followed by emit(&st, &cb)"
          },
          {
            "input": "user toggles 'Reconnect when the proxy stops responding'",
            "state": "settings_updated",
            "effect": "set",
            "evidence": "same anchor, copied a second time"
          },
          {
            "input": "either toggle while a connection is Running",
            "state": "restart_banner_quiet",
            "effect": "no-op",
            "evidence": "anchor 'snapshot.diverges_from(&self.settings, &current_rules, manual_nodes, subscriptions)' — only RuntimeConfigSnapshot fields can raise the banner, and health_check is deliberately not one of them"
          }
        ],
        "forbidden": [
          "adding health_check to RuntimeConfigSnapshot (anchor 'self.runtime_snapshot = Some(RuntimeConfigSnapshot {') — it would make each toggle raise the restart banner",
          "building the rows after the anchor '    drop(s);' — `s` is the borrow the .active(...) reads come from"
        ],
        "seeding": [
          "row_reflects_settings and settings_updated are reachable only through build_network_page with a live adw::PreferencesPage; there is no headless path, so both are proven by the manual step in the verification seam."
        ],
        "budgets": [
          "two rows, one PreferencesGroup, added after the group built at anchor '    let real_delay_group = adw::PreferencesGroup::builder()'",
          "zero new unit tests in this seam"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "In crates/ui/src/preferences/network.rs, before the anchor '    drop(s);', add a `adw::PreferencesGroup` titled 'Connection health' holding `health_check_enabled_row` (adw::SwitchRow, title 'Check connection health', .active(s.health_check.enabled)) and `health_check_failover_row` (title 'Reconnect when the proxy stops responding', .active(s.health_check.failover)); `page.add(&health_check_group);`.",
        "After the anchor '        real_delay_use_for_lowest_row.connect_active_notify(move |row| {' block, add two identically shaped blocks: `st.borrow_mut().health_check.enabled = row.is_active(); emit(&st, &cb);` and the same for `.failover`.",
        "Do not touch crates/ui/src/app.rs in this seam."
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-ui && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "h4",
      "taskIds": [
        "3.1",
        "3.2"
      ],
      "prev": "h2",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "seam": "ui-wiring",
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
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with candidate loop",
          "anchor": "                    report(ProcessState::Running, Some(meta.clone()));",
          "change": "after reporting Running, spawn the health monitor when settings.health_check.enabled; probe addr = listen_address (unspecified -> loopback) + settings.http_port, url/timeout from settings.real_delay (timeout_ms is u32 millis)"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "state_forwarder / monitor pause-reset",
          "anchor": "                    _ = mgr.wait_and_handle_exit() => {",
          "change": "monitor consumes a second `mgr.subscribe()` receiver so Starting pauses probing and Running resets the tracker; keep the existing forwarder untouched"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "exit paths (halt)",
          "anchor": "            halt(log_forwarder).await;",
          "change": "halt the monitor task alongside the forwarders here AND on every early `return` in the supervise loop / Stop branches, so no probe traffic survives handle.stop()"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg",
          "anchor": "    ProcessLogLine(u64, String),",
          "change": "add `ConnectionHealth(u64, Health)` variant (Health must derive Debug for the AppMsg derive)"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "log_forwarder",
          "anchor": "                        Ok(ProcessEvent::LogLine(line)) => {",
          "change": "when settings.backend.backend_type == BackendType::Xray, feed line.content into DnsFailureWindow and emit AppMsg::DnsHealth(generation, bool) on change; other backends unchanged"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "log_sender capture",
          "anchor": "            let log_sender = sender.clone();",
          "change": "move the backend type (Copy) into the forwarder task alongside log_sender"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg",
          "anchor": "    ProcessLogLine(u64, String),",
          "change": "add `DnsHealth(u64, bool)` variant"
        }
      ],
      "contract": {
        "states": [
          "monitor_absent",
          "monitor_probing",
          "monitor_paused",
          "monitor_halted",
          "app_health_none",
          "app_health_healthy",
          "app_health_unhealthy",
          "app_dns_failing",
          "failover_armed",
          "failover_blocked"
        ],
        "transitions": [
          {
            "input": "candidate reported Running with settings.health_check.enabled == true",
            "state": "monitor_probing",
            "effect": "set",
            "evidence": "anchor '                    report(ProcessState::Running, Some(meta.clone()));' — the monitor is spawned immediately after it, beside the two existing forwarders"
          },
          {
            "input": "candidate reported Running with settings.health_check.enabled == false",
            "state": "monitor_absent",
            "effect": "no-op",
            "evidence": "requirement: 'When health checks are disabled, no probe SHALL be sent.'"
          },
          {
            "input": "StateChanged { to: ProcessState::Starting } on the monitor's own receiver",
            "state": "monitor_paused",
            "effect": "forced",
            "evidence": "anchor 'let _ = self.state.transition(ProcessState::Starting, self.current_connection.clone());' in handle_unexpected_exit — the respawn relay; the monitor calls HealthTracker::reset() and stops probing"
          },
          {
            "input": "StateChanged { to: ProcessState::Running } on the monitor's own receiver",
            "state": "monitor_probing",
            "effect": "forced",
            "evidence": "requirement: 'Probing SHALL pause while the backend is being respawned and restart from a healthy state when it is `Running` again.'"
          },
          {
            "input": "broadcast::error::RecvError::Lagged on the monitor's receiver",
            "state": "monitor_probing",
            "effect": "no-op",
            "evidence": "anchor 'Err(broadcast::error::RecvError::Lagged(_)) => continue,' — the existing forwarders' handling; a busy log burst must not kill the monitor"
          },
          {
            "input": "ConnectionCmd::Stop arm of the supervision loop",
            "state": "monitor_halted",
            "effect": "forced",
            "evidence": "anchor '                        halt(state_forwarder).await;' inside the Some(ConnectionCmd::Stop) arm — today this path leaks log_forwarder; the replacement halts every spawned task"
          },
          {
            "input": "the '_ =>' arm after mgr.wait_and_handle_exit()",
            "state": "monitor_halted",
            "effect": "forced",
            "evidence": "anchor '                                halt(state_forwarder).await;' inside the `_ =>` arm — the second leaking path"
          },
          {
            "input": "supervision loop breaks on ProcessState::Error (candidate give-up)",
            "state": "monitor_halted",
            "effect": "forced",
            "evidence": "anchor '            halt(log_forwarder).await;' — the only path that halts both today"
          },
          {
            "input": "HealthTracker::record returns Some(health)",
            "state": "app_health_unhealthy or app_health_healthy",
            "effect": "set",
            "evidence": "requirement: 'After 3 consecutive failures the session SHALL be unhealthy' — the monitor emits AppMsg::ConnectionHealth(generation, health)"
          },
          {
            "input": "log line while settings.backend.backend_type == BackendType::Xray and DnsFailureWindow::is_failing flips",
            "state": "app_dns_failing",
            "effect": "set",
            "evidence": "anchor '                        Ok(ProcessEvent::LogLine(line)) => {' — emits AppMsg::DnsHealth(generation, bool) beside the existing AppMsg::ProcessLogLine"
          },
          {
            "input": "the same log lines while backend_type is SingBox or V2ray",
            "state": "app_dns_failing",
            "effect": "no-op",
            "evidence": "requirement: 'Other backends SHALL NOT be classified from their log output.'"
          },
          {
            "input": "AppMsg::ConnectionHealth or AppMsg::DnsHealth carrying a superseded generation",
            "state": "app_health_none",
            "effect": "no-op",
            "evidence": "anchor '                if !is_current_generation(generation, self.connection_generation) {' in the AppMsg::ProcessLogLine arm — the same guard"
          },
          {
            "input": "App::apply_state with any state other than ProcessState::Running",
            "state": "app_health_none",
            "effect": "clear",
            "evidence": "anchor '        self.process_state = state.clone();' — health and dns_failing are cleared here so a stale Unhealthy can never colour a Starting"
          },
          {
            "input": "status_texts with Running, metadata present, health Unhealthy(reason)",
            "state": "app_health_unhealthy",
            "effect": "set",
            "evidence": "requirement: 'An unhealthy session SHALL show `Proxy not responding` as the status text, with the connection details followed by the last probe failure.'"
          },
          {
            "input": "status_texts with Running, metadata present, health Healthy, dns_failing true",
            "state": "app_dns_failing",
            "effect": "set",
            "evidence": "requirement: 'A healthy session whose DNS through the proxy is marked failing SHALL show `Connected` with the details prefixed by `DNS via proxy failing`.'"
          },
          {
            "input": "status_texts with Running, metadata present, health Healthy, dns_failing false",
            "state": "app_health_healthy",
            "effect": "no-op",
            "evidence": "requirement: 'A healthy session without the DNS mark SHALL show the existing connected text.' — anchor '                (\"Connected\".to_string(), details)'"
          },
          {
            "input": "status_texts with Starting while a stale Unhealthy is still in the view struct",
            "state": "app_health_none",
            "effect": "no-op",
            "evidence": "sprint arm table: health is read only in the Running arms — anchor '            (ProcessState::Starting, _) => (\"Connecting…\".to_string(), \"Resolving nodes\".into()),'"
          },
          {
            "input": "Unhealthy transition while health_check.failover is true, session_target is None and health_failovers < MAX_AUTO_RECONNECTS",
            "state": "failover_armed",
            "effect": "set",
            "evidence": "requirement: 'When it is on and a session started by the configured strategy becomes unhealthy, the system SHALL disconnect and reconnect using the configured strategy with the unhealthy node excluded from that attempt's candidates.'"
          },
          {
            "input": "Unhealthy transition while health_check.failover is false",
            "state": "failover_blocked",
            "effect": "no-op",
            "evidence": "requirement scenario 'Failover off by default'"
          },
          {
            "input": "Unhealthy transition while session_target is Some(_)",
            "state": "failover_blocked",
            "effect": "no-op",
            "evidence": "requirement: 'A session started by a direct connection to a chosen node SHALL NOT fail over for health, regardless of the preference.'"
          },
          {
            "input": "Unhealthy transition while health_failovers == MAX_AUTO_RECONNECTS",
            "state": "failover_blocked",
            "effect": "no-op",
            "evidence": "requirement: 'The system SHALL perform at most 3 consecutive health failovers without a successful probe in between' — anchor 'const MAX_AUTO_RECONNECTS: u32 = 3;'"
          },
          {
            "input": "terminal Stopped arriving while health_reconnect_pending is true",
            "state": "failover_armed",
            "effect": "forced",
            "evidence": "requirement: 'the system SHALL disconnect and reconnect using the configured strategy' — dispatches AppMsg::Connect(ConnectOrigin::AutoReconnect) directly, not through the reconnect_pending path at anchor '                    sender.input(reconnect_msg(self.session_target, ConnectOrigin::Restart));' which would force ConnectOrigin::Restart"
          },
          {
            "input": "AppMsg::Connect handler after planner.plan",
            "state": "failover_armed",
            "effect": "clear",
            "evidence": "anchor '                let candidates = planner.plan(&subscriptions, &manual_nodes);' — self.excluded_node.take() removes the node for this attempt only"
          },
          {
            "input": "AppMsg::Connect or AppMsg::ConnectToNode with cancels_auto_reconnect(origin) true, or AppMsg::Disconnect",
            "state": "failover_blocked",
            "effect": "clear",
            "evidence": "requirement: 'a user-initiated Connect, direct connect, or Disconnect SHALL reset that count' — anchor 'fn cancels_auto_reconnect(origin: ConnectOrigin) -> bool {'"
          },
          {
            "input": "AppMsg::ConnectionHealth carrying Health::Healthy for the current generation",
            "state": "failover_blocked",
            "effect": "clear",
            "evidence": "requirement: 'a successful probe ... SHALL reset that count' — health_failovers = 0"
          },
          {
            "input": "AppMsg::Disconnect issued by the health failover (health_reconnect_pending true)",
            "state": "failover_armed",
            "effect": "no-op",
            "evidence": "crates/ui/src/app.rs Disconnect arm calls cancel_auto_reconnect() on every Disconnect; the self-issued stop must keep excluded_node and health_failovers"
          },
          {
            "input": "recheck tick with no counted line in the last 60 s",
            "state": "dns_clear",
            "effect": "set",
            "evidence": "emits AppMsg::DnsHealth(generation, false); DnsFailureWindow::is_failing(now) is false once DNS_WINDOW has elapsed since the newest counted line"
          }
        ],
        "forbidden": [
          "resetting health_failovers inside cancel_auto_reconnect (anchor '    fn cancel_auto_reconnect(&mut self) {') — it runs on every ProcessState::Running at anchor '                            self.cancel_auto_reconnect();', which would refill the budget after each failover and allow an unbounded loop",
          "reusing auto_reconnect_attempts (anchor '    auto_reconnect_attempts: u32,') as the health failover counter, for the same reason",
          "routing the health failover through self.reconnect_pending — the path at anchor '                    sender.input(reconnect_msg(self.session_target, ConnectOrigin::Restart));' hardcodes ConnectOrigin::Restart, and cancels_auto_reconnect(Restart) is true, so the Connect arm would clear excluded_node and reset health_failovers before the plan is filtered",
          "reading self.connection_status for the excluded node after dispatching AppMsg::Disconnect — the node_ref must be captured in the ConnectionHealth handler first",
          "spawning the monitor with the same broadcast receiver the state_forwarder holds — it takes a second mgr.subscribe(); the forwarder at anchor '            let mut state_rx = mgr.subscribe();' stays untouched",
          "returning from any post-Running exit path of the supervision loop without halting the monitor — a surviving monitor keeps connecting to the proxy port after handle.stop(), which this seam's own test asserts against",
          "probing at all when settings.health_check.enabled is false",
          "adding health_check to RuntimeConfigSnapshot",
          "hardcoding 10 s / 30 s inside the monitor body with no test-visible override — the harness cannot wait 10 s inside RECV_TIMEOUT = 20 s for a three-failure streak"
        ],
        "seeding": [
          "ProcessState::Running for a connection: only `connect(&stub, settings, vec![candidate(addr)])` (or the new connect_with_health) followed by `next_state(&rx)`. Never by constructing a ProcessState or emitting AppMsg by hand.",
          "monitor_paused: only a stub script that exits on its own so ProcessManager::handle_unexpected_exit respawns it (the existing crash-respawn stub pattern in crates/ui/src/connection.rs tests). Never by publishing a StateChanged event directly.",
          "monitor_halted: only `handle.stop()` followed by `assert_nothing_after_terminal(&rx)` plus an assertion that the fake proxy listener accepted no further connection.",
          "the probe endpoint: bind a tokio::net::TcpListener on 127.0.0.1:0, read its port, and pass that port as `http` to `v2ray_settings(socks, http, \"127.0.0.1\", false)`; the listener counts accepts and switches its canned reply from 204 to 502 on command.",
          "app_health_* and failover_* states: assert on the pure helpers only — `status_texts`, `should_announce`, `health_failover_allowed`, `exclude_candidate`. crates/ui/src/app.rs has no headless harness for App::update (its tests build fixtures like `snapshot(backend, tun_enabled)` and `session_target_node()` and call free functions), so no test constructs an App or drives a message through it.",
          "the xray DNS path: a stub script that prints 25 `app/dns: failed to retrieve response` lines, run with a settings value whose backend_type is Xray, drained with `drain(&rx)`; the sing-box counterpart uses `singbox_settings()` with the same script."
        ],
        "budgets": [
          "HEALTH_INITIAL_DELAY = Duration::from_secs(10)",
          "HEALTH_INTERVAL = Duration::from_secs(30)",
          "per-probe timeout = Duration::from_millis(u64::from(settings.real_delay.timeout_ms)) — the field is u32 milliseconds",
          "FAILURE_THRESHOLD = 3 consecutive failures before AppMsg::ConnectionHealth(_, Health::Unhealthy(_))",
          "MAX_AUTO_RECONNECTS = 3 health failovers",
          "test timing override: HealthTiming { initial_delay: Duration::from_millis(50), interval: Duration::from_millis(100) } so a three-failure streak lands well inside RECV_TIMEOUT = 20 s",
          "existing connection tests keep HealthTiming::default(), whose 10 s first probe never fires inside their lifetime",
          "DNS_RECHECK_INTERVAL: Duration::from_secs(10) — the timer tick that re-evaluates DnsFailureWindow::is_failing so the 60 s clear edge fires without a further log line"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST in crates/ui/src/connection.rs `mod tests`: `health_monitor_reports_unhealthy_then_healthy` (fake proxy listener; probes start failing -> exactly one AppMsg::ConnectionHealth(GENERATION, Health::Unhealthy(_)); restored -> exactly one Health::Healthy), `health_monitor_is_not_spawned_when_disabled` (health_check.enabled = false -> listener records zero accepts), `health_monitor_stops_with_the_connection` (handle.stop(); assert_nothing_after_terminal plus zero further accepts on the listener), `xray_dns_burst_reports_dns_health` (stub prints 25 matching lines -> exactly one AppMsg::DnsHealth(GENERATION, true)), `singbox_dns_burst_reports_nothing` (same stub output under singbox_settings() -> no AppMsg::DnsHealth).",
        "Add `HealthTiming { pub initial_delay: Duration, pub interval: Duration }` to crates/ui/src/connection.rs with a Default of 10 s / 30 s, add `pub health_timing: HealthTiming` to ConnectionRequest, destructure it in spawn_with beside `host_has_ipv6`, set `HealthTiming::default()` at the app.rs construction site, and add a `connect_with_health` test helper that overrides it.",
        "Replace the ad-hoc halts in the supervision loop with one helper that halts every spawned task (state_forwarder, log_forwarder, and the monitor when present) and call it on all three post-Running exit paths: the Some(ConnectionCmd::Stop) arm, the `_ =>` arm after mgr.wait_and_handle_exit(), and the fall-through after the loop breaks.",
        "Spawn the health monitor after the anchor '                    report(ProcessState::Running, Some(meta.clone()));' when effective_settings.health_check.enabled: it takes a second `mgr.subscribe()`, sleeps HealthTiming::initial_delay, then loops on a tokio::select! over the state receiver and an interval tick; Starting pauses probing and calls HealthTracker::reset(), Running resumes, Lagged continues, Closed breaks. Each tick calls v2ray_rs_subscription::probe_via_http_proxy(addr, &settings.real_delay.test_url, timeout) and emits AppMsg::ConnectionHealth(generation, health) for every Some(_) the tracker returns.",
        "Compute the probe address from the shared helper: `effective_settings.local_endpoint(effective_settings.http_port)` when confirm-backend-ready-before-running is already merged; otherwise add that helper to crates/core/src/models/settings.rs beside the anchor '    pub fn validate_listen_address(addr: &str) -> Result<(), ValidationError> {' with its four unit cases (127.0.0.1, 0.0.0.0 -> 127.0.0.1, :: -> ::1, 192.168.1.10).",
        "Feed the DNS window from the log forwarder: move the Copy backend_type into the task at the anchor '            let log_sender = sender.clone();', and inside the arm at anchor '                        Ok(ProcessEvent::LogLine(line)) => {' call DnsFailureWindow::observe when the backend is BackendType::Xray, emitting AppMsg::DnsHealth(generation, flag) only when is_failing flips.",
        "Add `ConnectionHealth(u64, Health)` and `DnsHealth(u64, bool)` to AppMsg after the anchor '    ProcessLogLine(u64, String),'.",
        "Add the two `AppMsg::ConnectionHealth` / `AppMsg::DnsHealth` match arms in the SAME chunk that adds the variants — `App::update`'s match at crates/ui/src/app.rs has no wildcard arm, so adding a variant without its arm is E0004 and h4's own `make test-ui` fails. h4 lands them behind the existing `is_current_generation` guard storing the value; h5 fills in the announce/failover behavior.",
        "Drive the DNS clear edge from a timer, not from log lines: the window is only evaluated inside the LogLine arm, so once a burst stops no further line arrives, `is_failing` is never re-evaluated and `DnsHealth(_, false)` is never emitted — the 60 s clear requirement would be unimplemented. Re-evaluate on a `tokio::time::interval(DNS_RECHECK_INTERVAL)` tick in the forwarder select (or on the health monitor's own interval tick).",
        "TEST FIRST: add `dns_window_clears_after_the_recheck_tick` — DnsFailureWindow takes `now` as a parameter, so drive 20 matching lines then a tick 61 s later and assert is_failing flips false; at the forwarder level ride the existing xray burst test with an injected short recheck interval and assert exactly one `AppMsg::DnsHealth(GENERATION, false)`"
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-ui && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "h5",
      "taskIds": [
        "4.1",
        "4.3"
      ],
      "prev": "h4",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "seam": "ui-wiring",
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
          "symbol": "App struct",
          "anchor": "    connection_status: Option<ConnectionMetadata>,",
          "change": "add `health: Option<Health>` and `dns_failing: bool` fields (cleared on any non-Running state)"
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::init model literal",
          "anchor": "            session_target: None,",
          "change": "initialize the two new fields in the model literal"
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "status_texts(&StatusView) (shared with log-connection-decisions)",
          "anchor": "    fn update_status_labels(&self) {",
          "change": "Extend the sprint's shared `StatusView { state, prev_state, meta, origin, attempt }` with `health: Option<Health>` and `dns_failing: bool`, and the pure `status_texts(&StatusView) -> (String, String)` with the three Running arms in the sprint's precedence order. `log-connection-decisions` introduces the struct and the function (its tasks.md 4.1 now names `status_texts(&StatusView)`, not `status_primary`); this change lands last and only adds fields. If it is absent, introduce it here with `state`, `meta`, `health`, `dns_failing` and leave room for `prev_state`, `origin`, `attempt`."
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "status_texts(&StatusView) (shared with log-connection-decisions)",
          "anchor": "                (\"Connected\".to_string(), details)",
          "change": "Extend the sprint's shared `StatusView { state, prev_state, meta, origin, attempt }` with `health: Option<Health>` and `dns_failing: bool`, and the pure `status_texts(&StatusView) -> (String, String)` with the three Running arms in the sprint's precedence order. `log-connection-decisions` introduces the struct and the function (its tasks.md 4.1 now names `status_texts(&StatusView)`, not `status_primary`); this change lands last and only adds fields. If it is absent, introduce it here with `state`, `meta`, `health`, `dns_failing` and leave room for `prev_state`, `origin`, `attempt`."
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::ConnectionHealth / DnsHealth handlers",
          "anchor": "            AppMsg::ProcessLogLine(generation, line) => {",
          "change": "add handlers next to this one, gated by the same `is_current_generation(generation, self.connection_generation)` guard; store health/dns_failing then update_status_labels()"
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::apply_state",
          "anchor": "        self.process_state = state.clone();",
          "change": "clear health and dns_failing whenever the new state is not Running, so Starting cannot show stale health"
        },
        {
          "task": "4.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "App struct",
          "anchor": "    auto_reconnect_attempts: u32,",
          "change": "add `health_failovers: u32` and `excluded_node: Option<ConnectionNodeRef>` (one-attempt exclusion); initialize both in the model literal"
        },
        {
          "task": "4.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::Connect handler",
          "anchor": "                let candidates = planner.plan(&subscriptions, &manual_nodes);",
          "change": "after planning, drop `self.excluded_node.take()` from candidates for this attempt only; empty-after-exclusion falls through to the existing empty-candidates toast"
        },
        {
          "task": "4.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "health failover trigger",
          "anchor": "            AppMsg::ProcessLogLine(generation, line) => {",
          "change": "in the ConnectionHealth handler added next to this arm: on Unhealthy, if health_failover_allowed(settings.health_check.failover, self.session_target, self.health_failovers) set excluded_node from connection_status.node_ref, increment health_failovers, set reconnect_pending and dispatch Disconnect then Connect(ConnectOrigin::AutoReconnect)"
        },
        {
          "task": "4.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "health_failover_allowed / exclude_candidate (new pure fns)",
          "anchor": "struct SessionTarget {",
          "change": "add the two pure helpers beside SessionTarget: allowed only when enabled, session_target is None (direct target -> false) and count < MAX_AUTO_RECONNECTS"
        },
        {
          "task": "4.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "budget constant",
          "anchor": "const MAX_AUTO_RECONNECTS: u32 = 3;",
          "change": "reuse this cap for health failovers; do NOT reuse auto_reconnect_attempts (cancel_auto_reconnect runs on every Running and would refill it)"
        },
        {
          "task": "4.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "counter reset",
          "anchor": "    fn cancel_auto_reconnect(&mut self) {",
          "change": "do NOT reset health_failovers here; reset it explicitly in the AppMsg::Connect / ConnectToNode / Disconnect arms (user actions) and on a Healthy transition"
        },
        {
          "task": "4.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::Connect user reset",
          "anchor": "            AppMsg::Connect(origin) => {",
          "change": "reset health_failovers and clear excluded_node when cancels_auto_reconnect(origin) is true (User/Restart), not for AutoReconnect"
        },
        {
          "task": "4.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::Disconnect",
          "anchor": "            AppMsg::Disconnect => {",
          "change": "reset health_failovers and clear excluded_node"
        },
        {
          "task": "4.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::ConnectToNode",
          "anchor": "            AppMsg::ConnectToNode(target, origin) => {",
          "change": "reset health_failovers on a user direct connect (the session_target it sets already blocks failover)"
        }
      ],
      "contract": {
        "states": [
          "monitor_absent",
          "monitor_probing",
          "monitor_paused",
          "monitor_halted",
          "app_health_none",
          "app_health_healthy",
          "app_health_unhealthy",
          "app_dns_failing",
          "failover_armed",
          "failover_blocked"
        ],
        "transitions": [
          {
            "input": "candidate reported Running with settings.health_check.enabled == true",
            "state": "monitor_probing",
            "effect": "set",
            "evidence": "anchor '                    report(ProcessState::Running, Some(meta.clone()));' — the monitor is spawned immediately after it, beside the two existing forwarders"
          },
          {
            "input": "candidate reported Running with settings.health_check.enabled == false",
            "state": "monitor_absent",
            "effect": "no-op",
            "evidence": "requirement: 'When health checks are disabled, no probe SHALL be sent.'"
          },
          {
            "input": "StateChanged { to: ProcessState::Starting } on the monitor's own receiver",
            "state": "monitor_paused",
            "effect": "forced",
            "evidence": "anchor 'let _ = self.state.transition(ProcessState::Starting, self.current_connection.clone());' in handle_unexpected_exit — the respawn relay; the monitor calls HealthTracker::reset() and stops probing"
          },
          {
            "input": "StateChanged { to: ProcessState::Running } on the monitor's own receiver",
            "state": "monitor_probing",
            "effect": "forced",
            "evidence": "requirement: 'Probing SHALL pause while the backend is being respawned and restart from a healthy state when it is `Running` again.'"
          },
          {
            "input": "broadcast::error::RecvError::Lagged on the monitor's receiver",
            "state": "monitor_probing",
            "effect": "no-op",
            "evidence": "anchor 'Err(broadcast::error::RecvError::Lagged(_)) => continue,' — the existing forwarders' handling; a busy log burst must not kill the monitor"
          },
          {
            "input": "ConnectionCmd::Stop arm of the supervision loop",
            "state": "monitor_halted",
            "effect": "forced",
            "evidence": "anchor '                        halt(state_forwarder).await;' inside the Some(ConnectionCmd::Stop) arm — today this path leaks log_forwarder; the replacement halts every spawned task"
          },
          {
            "input": "the '_ =>' arm after mgr.wait_and_handle_exit()",
            "state": "monitor_halted",
            "effect": "forced",
            "evidence": "anchor '                                halt(state_forwarder).await;' inside the `_ =>` arm — the second leaking path"
          },
          {
            "input": "supervision loop breaks on ProcessState::Error (candidate give-up)",
            "state": "monitor_halted",
            "effect": "forced",
            "evidence": "anchor '            halt(log_forwarder).await;' — the only path that halts both today"
          },
          {
            "input": "HealthTracker::record returns Some(health)",
            "state": "app_health_unhealthy or app_health_healthy",
            "effect": "set",
            "evidence": "requirement: 'After 3 consecutive failures the session SHALL be unhealthy' — the monitor emits AppMsg::ConnectionHealth(generation, health)"
          },
          {
            "input": "log line while settings.backend.backend_type == BackendType::Xray and DnsFailureWindow::is_failing flips",
            "state": "app_dns_failing",
            "effect": "set",
            "evidence": "anchor '                        Ok(ProcessEvent::LogLine(line)) => {' — emits AppMsg::DnsHealth(generation, bool) beside the existing AppMsg::ProcessLogLine"
          },
          {
            "input": "the same log lines while backend_type is SingBox or V2ray",
            "state": "app_dns_failing",
            "effect": "no-op",
            "evidence": "requirement: 'Other backends SHALL NOT be classified from their log output.'"
          },
          {
            "input": "AppMsg::ConnectionHealth or AppMsg::DnsHealth carrying a superseded generation",
            "state": "app_health_none",
            "effect": "no-op",
            "evidence": "anchor '                if !is_current_generation(generation, self.connection_generation) {' in the AppMsg::ProcessLogLine arm — the same guard"
          },
          {
            "input": "App::apply_state with any state other than ProcessState::Running",
            "state": "app_health_none",
            "effect": "clear",
            "evidence": "anchor '        self.process_state = state.clone();' — health and dns_failing are cleared here so a stale Unhealthy can never colour a Starting"
          },
          {
            "input": "status_texts with Running, metadata present, health Unhealthy(reason)",
            "state": "app_health_unhealthy",
            "effect": "set",
            "evidence": "requirement: 'An unhealthy session SHALL show `Proxy not responding` as the status text, with the connection details followed by the last probe failure.'"
          },
          {
            "input": "status_texts with Running, metadata present, health Healthy, dns_failing true",
            "state": "app_dns_failing",
            "effect": "set",
            "evidence": "requirement: 'A healthy session whose DNS through the proxy is marked failing SHALL show `Connected` with the details prefixed by `DNS via proxy failing`.'"
          },
          {
            "input": "status_texts with Running, metadata present, health Healthy, dns_failing false",
            "state": "app_health_healthy",
            "effect": "no-op",
            "evidence": "requirement: 'A healthy session without the DNS mark SHALL show the existing connected text.' — anchor '                (\"Connected\".to_string(), details)'"
          },
          {
            "input": "status_texts with Starting while a stale Unhealthy is still in the view struct",
            "state": "app_health_none",
            "effect": "no-op",
            "evidence": "sprint arm table: health is read only in the Running arms — anchor '            (ProcessState::Starting, _) => (\"Connecting…\".to_string(), \"Resolving nodes\".into()),'"
          },
          {
            "input": "Unhealthy transition while health_check.failover is true, session_target is None and health_failovers < MAX_AUTO_RECONNECTS",
            "state": "failover_armed",
            "effect": "set",
            "evidence": "requirement: 'When it is on and a session started by the configured strategy becomes unhealthy, the system SHALL disconnect and reconnect using the configured strategy with the unhealthy node excluded from that attempt's candidates.'"
          },
          {
            "input": "Unhealthy transition while health_check.failover is false",
            "state": "failover_blocked",
            "effect": "no-op",
            "evidence": "requirement scenario 'Failover off by default'"
          },
          {
            "input": "Unhealthy transition while session_target is Some(_)",
            "state": "failover_blocked",
            "effect": "no-op",
            "evidence": "requirement: 'A session started by a direct connection to a chosen node SHALL NOT fail over for health, regardless of the preference.'"
          },
          {
            "input": "Unhealthy transition while health_failovers == MAX_AUTO_RECONNECTS",
            "state": "failover_blocked",
            "effect": "no-op",
            "evidence": "requirement: 'The system SHALL perform at most 3 consecutive health failovers without a successful probe in between' — anchor 'const MAX_AUTO_RECONNECTS: u32 = 3;'"
          },
          {
            "input": "terminal Stopped arriving while health_reconnect_pending is true",
            "state": "failover_armed",
            "effect": "forced",
            "evidence": "requirement: 'the system SHALL disconnect and reconnect using the configured strategy' — dispatches AppMsg::Connect(ConnectOrigin::AutoReconnect) directly, not through the reconnect_pending path at anchor '                    sender.input(reconnect_msg(self.session_target, ConnectOrigin::Restart));' which would force ConnectOrigin::Restart"
          },
          {
            "input": "AppMsg::Connect handler after planner.plan",
            "state": "failover_armed",
            "effect": "clear",
            "evidence": "anchor '                let candidates = planner.plan(&subscriptions, &manual_nodes);' — self.excluded_node.take() removes the node for this attempt only"
          },
          {
            "input": "AppMsg::Connect or AppMsg::ConnectToNode with cancels_auto_reconnect(origin) true, or AppMsg::Disconnect",
            "state": "failover_blocked",
            "effect": "clear",
            "evidence": "requirement: 'a user-initiated Connect, direct connect, or Disconnect SHALL reset that count' — anchor 'fn cancels_auto_reconnect(origin: ConnectOrigin) -> bool {'"
          },
          {
            "input": "AppMsg::ConnectionHealth carrying Health::Healthy for the current generation",
            "state": "failover_blocked",
            "effect": "clear",
            "evidence": "requirement: 'a successful probe ... SHALL reset that count' — health_failovers = 0"
          },
          {
            "input": "AppMsg::Disconnect issued by the health failover (health_reconnect_pending true)",
            "state": "failover_armed",
            "effect": "no-op",
            "evidence": "crates/ui/src/app.rs Disconnect arm calls cancel_auto_reconnect() on every Disconnect; the self-issued stop must keep excluded_node and health_failovers"
          }
        ],
        "forbidden": [
          "resetting health_failovers inside cancel_auto_reconnect (anchor '    fn cancel_auto_reconnect(&mut self) {') — it runs on every ProcessState::Running at anchor '                            self.cancel_auto_reconnect();', which would refill the budget after each failover and allow an unbounded loop",
          "reusing auto_reconnect_attempts (anchor '    auto_reconnect_attempts: u32,') as the health failover counter, for the same reason",
          "routing the health failover through self.reconnect_pending — the path at anchor '                    sender.input(reconnect_msg(self.session_target, ConnectOrigin::Restart));' hardcodes ConnectOrigin::Restart, and cancels_auto_reconnect(Restart) is true, so the Connect arm would clear excluded_node and reset health_failovers before the plan is filtered",
          "reading self.connection_status for the excluded node after dispatching AppMsg::Disconnect — the node_ref must be captured in the ConnectionHealth handler first",
          "spawning the monitor with the same broadcast receiver the state_forwarder holds — it takes a second mgr.subscribe(); the forwarder at anchor '            let mut state_rx = mgr.subscribe();' stays untouched",
          "returning from any post-Running exit path of the supervision loop without halting the monitor — a surviving monitor keeps connecting to the proxy port after handle.stop(), which this seam's own test asserts against",
          "probing at all when settings.health_check.enabled is false",
          "adding health_check to RuntimeConfigSnapshot",
          "hardcoding 10 s / 30 s inside the monitor body with no test-visible override — the harness cannot wait 10 s inside RECV_TIMEOUT = 20 s for a three-failure streak"
        ],
        "seeding": [
          "ProcessState::Running for a connection: only `connect(&stub, settings, vec![candidate(addr)])` (or the new connect_with_health) followed by `next_state(&rx)`. Never by constructing a ProcessState or emitting AppMsg by hand.",
          "monitor_paused: only a stub script that exits on its own so ProcessManager::handle_unexpected_exit respawns it (the existing crash-respawn stub pattern in crates/ui/src/connection.rs tests). Never by publishing a StateChanged event directly.",
          "monitor_halted: only `handle.stop()` followed by `assert_nothing_after_terminal(&rx)` plus an assertion that the fake proxy listener accepted no further connection.",
          "the probe endpoint: bind a tokio::net::TcpListener on 127.0.0.1:0, read its port, and pass that port as `http` to `v2ray_settings(socks, http, \"127.0.0.1\", false)`; the listener counts accepts and switches its canned reply from 204 to 502 on command.",
          "app_health_* and failover_* states: assert on the pure helpers only — `status_texts`, `should_announce`, `health_failover_allowed`, `exclude_candidate`. crates/ui/src/app.rs has no headless harness for App::update (its tests build fixtures like `snapshot(backend, tun_enabled)` and `session_target_node()` and call free functions), so no test constructs an App or drives a message through it.",
          "the xray DNS path: a stub script that prints 25 `app/dns: failed to retrieve response` lines, run with a settings value whose backend_type is Xray, drained with `drain(&rx)`; the sing-box counterpart uses `singbox_settings()` with the same script."
        ],
        "budgets": [
          "HEALTH_INITIAL_DELAY = Duration::from_secs(10)",
          "HEALTH_INTERVAL = Duration::from_secs(30)",
          "per-probe timeout = Duration::from_millis(u64::from(settings.real_delay.timeout_ms)) — the field is u32 milliseconds",
          "FAILURE_THRESHOLD = 3 consecutive failures before AppMsg::ConnectionHealth(_, Health::Unhealthy(_))",
          "MAX_AUTO_RECONNECTS = 3 health failovers",
          "test timing override: HealthTiming { initial_delay: Duration::from_millis(50), interval: Duration::from_millis(100) } so a three-failure streak lands well inside RECV_TIMEOUT = 20 s",
          "existing connection tests keep HealthTiming::default(), whose 10 s first probe never fires inside their lifetime"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST in crates/ui/src/app.rs `mod tests`: `status_texts_unhealthy_says_proxy_not_responding`, `status_texts_dns_failing_prefixes_details`, `status_texts_healthy_matches_todays_connected_text`, `status_texts_starting_ignores_stale_health`, `health_failover_allowed_rejects_direct_target`, `health_failover_allowed_rejects_exhausted_budget`, `health_failover_allowed_rejects_disabled_preference`, `exclude_candidate_drops_only_the_named_node`.",
        "Add `health: Option<Health>`, `dns_failing: bool`, `health_failovers: u32`, `excluded_node: Option<ConnectionNodeRef>` and `health_reconnect_pending: bool` to the App struct and to the model literal at the anchor '            session_target: None,'.",
        "Extract the body of `fn update_status_labels(&self)` into a pure function over the sprint's StatusView struct, adding the `health` and `dns_failing` fields to it (the 4.1 site states the fallback if StatusView is absent), and implement the three Running arms in the sprint's precedence order.",
        "Clear health and dns_failing in App::apply_state whenever the new state is not ProcessState::Running, at the anchor '        self.process_state = state.clone();'.",
        "Extend the two message handlers h4 created beside the anchor '            AppMsg::ProcessLogLine(generation, line) => {', both behind the existing is_current_generation guard; the ConnectionHealth handler stores the health, resets health_failovers on Healthy, calls update_status_labels, and on an Unhealthy transition captures self.connection_status.as_ref().map(|m| m.node_ref) BEFORE deciding on failover.",
        "Add `fn health_failover_allowed(enabled: bool, session_target: Option<SessionTarget>, count: u32) -> bool` and `fn exclude_candidate(candidates: Vec<ConnectionCandidate>, excluded: Option<ConnectionNodeRef>) -> Vec<ConnectionCandidate>` beside the anchor 'struct SessionTarget {'.",
        "Wire the failover: on an armed Unhealthy set excluded_node, increment health_failovers, set health_reconnect_pending and dispatch AppMsg::Disconnect; in the terminal branch of AppMsg::ProcessStateConnection check health_reconnect_pending BEFORE the reconnect_after_stop check and dispatch AppMsg::Connect(ConnectOrigin::AutoReconnect); apply exclude_candidate after the anchor '                let candidates = planner.plan(&subscriptions, &manual_nodes);'; reset health_failovers and clear excluded_node in the Connect/ConnectToNode arms when cancels_auto_reconnect(origin) is true and in the Disconnect arm ONLY when `!self.health_reconnect_pending` (the failover issues its own Disconnect and must keep its exclusion and budget).",
        "Gate the Disconnect-arm reset on `!self.health_reconnect_pending`: the failover dispatches AppMsg::Disconnect itself, and the existing arm clears `excluded_node` and `health_failovers` (and calls cancel_auto_reconnect) on every Disconnect — so without the carve-out the exclusion is cleared before AppMsg::Connect filters the plan, the budget never accumulates, and the same dead node is re-picked without bound."
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-ui && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "h6",
      "taskIds": [
        "4.2"
      ],
      "prev": "h5",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "seam": "tray-notify",
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-tray",
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "4.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "should_announce (new pure fn)",
          "anchor": "fn active_nodes_available(subscriptions: &[Subscription], manual_nodes: &[ManualNode]) -> bool {",
          "change": "add `fn should_announce(prev: Option<&Health>, next: &Health) -> bool` near the other pure helpers; only a Healthy/None -> Unhealthy edge announces"
        },
        {
          "task": "4.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "toast on unhealthy",
          "anchor": "    fn show_toast(&self, msg: &str) {",
          "change": "reuse show_toast for `Proxy not responding: <reason>`; when settings.notifications_enabled also call the new tray notify entry point through TRAY_HANDLE"
        },
        {
          "task": "4.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "tray handle access",
          "anchor": "fn update_tray_notification_setting(enabled: bool) {",
          "change": "add a sibling free fn that locks TRAY_HANDLE and calls the new TrayHandle notify method (same `if let Ok(mut guard) ... and let Some(handle)` shape)"
        },
        {
          "task": "4.2",
          "file": "crates/tray/src/tray.rs",
          "symbol": "impl TrayHandle",
          "anchor": "    pub fn set_notifications_enabled(&mut self, enabled: bool) {",
          "change": "add `pub fn notify(&self, summary: &str, body: &str)` delegating to the Notifier; keep the enabled gate inside the Notifier"
        },
        {
          "task": "4.2",
          "file": "crates/tray/src/notification.rs",
          "symbol": "Notifier",
          "anchor": "    fn send(&self, summary: &str, body: &str) {",
          "change": "make a public wrapper (e.g. `pub fn notify(&self, summary, body)`) that checks `self.enabled` then calls send; `send` itself is already blocking (Notification::show) so the caller should spawn_blocking like TrayService does"
        }
      ],
      "contract": {
        "states": [
          "announced",
          "quiet"
        ],
        "transitions": [
          {
            "input": "should_announce(None, &Health::Unhealthy(reason))",
            "state": "announced",
            "effect": "set",
            "evidence": "requirement: 'When a session becomes unhealthy, the system SHALL show a notification in the main window'"
          },
          {
            "input": "should_announce(Some(&Health::Healthy), &Health::Unhealthy(reason))",
            "state": "announced",
            "effect": "set",
            "evidence": "requirement scenario 'First transition notifies'"
          },
          {
            "input": "should_announce(Some(&Health::Unhealthy(_)), &Health::Unhealthy(_))",
            "state": "quiet",
            "effect": "no-op",
            "evidence": "requirement: 'The system SHALL NOT repeat either while the session stays unhealthy.'"
          },
          {
            "input": "should_announce(_, &Health::Healthy)",
            "state": "quiet",
            "effect": "no-op",
            "evidence": "requirement names only the unhealthy transition as announced"
          },
          {
            "input": "an announced transition while settings.notifications_enabled is false",
            "state": "announced",
            "effect": "set",
            "evidence": "requirement: 'and, when desktop notifications are enabled, SHALL send a desktop notification' — the toast still shows, the desktop notification does not"
          }
        ],
        "forbidden": [
          "calling Notification::show on the GTK main thread — anchor '    fn send(&self, summary: &str, body: &str) {' is blocking; the tray service wraps its calls in tokio::task::spawn_blocking and the new path must do the same",
          "dropping the enabled gate — Notifier owns it (anchor '        if !self.enabled.load(Ordering::Relaxed) {' in on_state_change); the new public wrapper keeps the check inside the Notifier rather than at the call site",
          "holding the TRAY_HANDLE mutex across the blocking show call",
          "announcing from anywhere but the ConnectionHealth handler's transition edge"
        ],
        "seeding": [
          "announced / quiet: only by calling the pure `should_announce(prev, next)` with Health values built directly. No test drives a real notification: the desktop side needs a live org.freedesktop.Notifications service and is covered by the manual verification step."
        ],
        "budgets": [
          "at most one toast and one desktop notification per Healthy-or-None -> Unhealthy edge",
          "NOTIFICATION_TIMEOUT_MS = 5000, unchanged",
          "one new unit test in this seam"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "TEST FIRST in crates/ui/src/app.rs `mod tests`: `should_announce_fires_once_per_unhealthy_streak` covering all four rows of the transition table.",
        "Add `pub fn notify(&self, summary: &str, body: &str)` to Notifier in crates/tray/src/notification.rs, checking `self.enabled` then delegating to the existing private `send`.",
        "Add `pub fn notify(&self, summary: &str, body: &str)` to TrayHandle beside the anchor '    pub fn set_notifications_enabled(&mut self, enabled: bool) {' in crates/tray/src/tray.rs, delegating to the Notifier.",
        "Add `fn should_announce(prev: Option<&Health>, next: &Health) -> bool` near the anchor 'fn active_nodes_available(subscriptions: &[Subscription], manual_nodes: &[ManualNode]) -> bool {'.",
        "Add a free function beside the anchor 'fn update_tray_notification_setting(enabled: bool) {' that clones what it needs out of the TRAY_HANDLE lock, releases the lock, and runs the notify call inside tokio::task::spawn_blocking.",
        "In the AppMsg::ConnectionHealth handler added by the ui-wiring seam, call show_toast with `Proxy not responding: <reason>` on an announced edge, and the new notify path additionally when settings.notifications_enabled."
      ],
      "redTests": [],
      "redRun": "",
      "verify": "make test-tray && make test-ui && make lint",
      "coder": "rust-coder"
    },
    {
      "id": "h7",
      "taskIds": [
        "5.1",
        "5.2"
      ],
      "prev": "h6",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "seam": "verification",
      "shard": "",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-core",
        "v2ray-rs-subscription",
        "v2ray-rs-ui",
        "v2ray-rs-tray"
      ],
      "sites": [],
      "contract": {
        "states": [
          "workspace_green",
          "live_unhealthy_observed",
          "live_recovered"
        ],
        "transitions": [
          {
            "input": "TEST_TIMEOUT=10m make test",
            "state": "workspace_green",
            "effect": "set",
            "evidence": "Makefile anchor 'TEST := timeout $(TEST_TIMEOUT) $(CARGO) test' with 'TEST_ARGS := -- --test-threads=$(TEST_THREADS)'"
          },
          {
            "input": "live: nft rule dropping the connected node's address while a session runs",
            "state": "live_unhealthy_observed",
            "effect": "set",
            "evidence": "task 5.2 — status shows `Proxy not responding` with exactly one toast within about 2 minutes"
          },
          {
            "input": "live: the nft rule removed",
            "state": "live_recovered",
            "effect": "forced",
            "evidence": "task 5.2 — status returns to `Connected` with no toast"
          }
        ],
        "forbidden": [
          "a bare `cargo test` without the timeout and --test-threads cap — the Makefile targets carry both",
          "treating 5.2's absence as a failure of the change; it is marked manual"
        ],
        "seeding": [
          "workspace_green: only the Makefile targets below.",
          "live_unhealthy_observed / live_recovered: only a real xray backend with a real subscription and a root-installed nft rule in a dedicated test table, removed afterwards. Not reachable from any test harness."
        ],
        "budgets": [
          "workspace test wall clock: 10 minutes",
          "test threads: 4",
          "live detection window: about 2 minutes (10 s initial delay + 3 intervals of 30 s + probe timeouts)",
          "expected toasts during the live outage: exactly 1"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "Run `TEST_TIMEOUT=10m make test` and `make lint` and fix what they report.",
        "Perform the live xray check described in task 5.2 and record the observed status text, toast count and, with failover on, the node the reconnect picked. Report it as a manual result."
      ],
      "redTests": [],
      "redRun": "",
      "verify": "TEST_TIMEOUT=10m make test && make lint",
      "coder": "rust-coder"
    }
  ],
  "seams": [
    {
      "id": "settings-and-probe",
      "tasks": [
        "1.1",
        "1.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack has no red-stage agent in this pipeline, so the coder writes the tests first inside its own chunk. NO-TESTER-WAIVER: same reason — no Rust test-writer agent exists; tests are the first codeTasks of this seam. Adds HealthCheckSettings to AppSettings (serde default, legacy TOML tolerant) and a pure async probe `probe_via_http_proxy` in the subscription crate, which is the only crate below ui that carries reqwest (crates/ui/Cargo.toml has no reqwest dependency). Test files the coder may touch: the inline `mod tests` in crates/core/src/models/settings.rs and a new inline `mod tests` in crates/subscription/src/health.rs. No other file gets test changes.",
      "contract": {
        "states": [
          "health_check_defaults",
          "health_check_custom",
          "probe_ok",
          "probe_status_error",
          "probe_timeout",
          "probe_transport_error"
        ],
        "transitions": [
          {
            "input": "settings TOML carrying no [health_check] table",
            "state": "health_check_defaults",
            "effect": "set",
            "evidence": "anchor 'fn test_legacy_settings_toml_missing_real_delay_defaults() {' — the same shape, cloned for health_check"
          },
          {
            "input": "AppSettings::default()",
            "state": "health_check_defaults",
            "effect": "set",
            "evidence": "anchor '            real_delay: RealDelaySettings::default(),' in impl Default for AppSettings"
          },
          {
            "input": "toml::to_string then toml::from_str of an AppSettings with health_check.failover = true",
            "state": "health_check_custom",
            "effect": "no-op",
            "evidence": "anchor 'fn test_real_delay_settings_round_trip() {' — asserts full AppSettings equality after the round trip"
          },
          {
            "input": "proxy listener answers 'HTTP/1.1 204 No Content'",
            "state": "probe_ok",
            "effect": "set",
            "evidence": "requirement: 'A response with a status below 400 SHALL count as a success'"
          },
          {
            "input": "proxy listener answers 'HTTP/1.1 502 Bad Gateway'",
            "state": "probe_status_error",
            "effect": "set",
            "evidence": "requirement: 'A response with a status below 400 SHALL count as a success; a connection, TLS, or protocol error or a timeout SHALL count as a failure'"
          },
          {
            "input": "proxy listener accepts and never writes, probe timeout 300 ms",
            "state": "probe_timeout",
            "effect": "set",
            "evidence": "requirement: 'or a timeout SHALL count as a failure'"
          },
          {
            "input": "no listener bound at the proxy address",
            "state": "probe_transport_error",
            "effect": "set",
            "evidence": "requirement: 'a connection, TLS, or protocol error ... SHALL count as a failure'"
          }
        ],
        "forbidden": [
          "health_check added to RuntimeConfigSnapshot — verified absent from the literal at anchor 'self.runtime_snapshot = Some(RuntimeConfigSnapshot {'; adding it would make check_restart_required raise the restart banner on a toggle, contradicting task 4.4",
          "health_check declared before any scalar field of AppSettings — toml::to_string fails with ValueAfterTable; it must sit after 'pub real_delay: RealDelaySettings,', which is today the last field",
          "a socks:// proxy URL — the workspace reqwest pin 'reqwest = { version = \"0.13\", features = [\"rustls-no-provider\", \"stream\"], default-features = false }' has no `socks` feature; only http:// proxies are reachable",
          "classifying a timeout by matching reqwest::Error's Display text — the probe must branch on err.is_timeout() and emit its own message containing 'timed out'",
          "probe_via_http_proxy left crate-private — every module in crates/subscription/src/lib.rs is pub(crate), so without an explicit re-export the ui crate cannot call it"
        ],
        "seeding": [
          "health_check_defaults: only via toml::from_str on a TOML string with no [health_check] table, or AppSettings::default(). Never by writing the field on a constructed struct.",
          "health_check_custom: only via an AppSettings struct literal with '..AppSettings::default()', mirroring 'fn test_real_delay_settings_round_trip'.",
          "probe_ok / probe_status_error / probe_timeout: only by calling probe_via_http_proxy against a tokio::net::TcpListener bound to 127.0.0.1:0 that speaks raw HTTP/1.1, modelled on 'async fn spawn_mock_clash(responses: Vec<Option<u64>>) -> u16' in crates/subscription/src/real_delay.rs. The probe URL in these tests MUST use the http:// scheme: with an https:// URL reqwest sends CONNECT to the proxy, which the raw listener does not implement.",
          "probe_transport_error: bind a listener with pick_free_loopback_port()'s pattern, drop it, then probe that port."
        ],
        "budgets": [
          "probe timeout in the timeout test: 300 ms",
          "success status threshold: status < 400",
          "health_check defaults: enabled = true, failover = false",
          "three probe tests total (204, 502, no-answer); a fourth for the unbound port is optional"
        ]
      },
      "codeTasks": [
        "TEST FIRST: in crates/core/src/models/settings.rs `mod tests`, add `test_legacy_settings_toml_missing_health_check_defaults` (TOML string identical to the one at anchor 'fn test_legacy_settings_toml_missing_real_delay_defaults() {', asserting settings.health_check.enabled == true and settings.health_check.failover == false) and `test_health_check_settings_round_trip` (AppSettings with `health_check: HealthCheckSettings { enabled: false, failover: true }`, `..AppSettings::default()`, toml::to_string -> from_str, assert_eq on the whole AppSettings).",
        "TEST FIRST: create crates/subscription/src/health.rs with `mod tests` holding `probe_succeeds_on_204`, `probe_fails_on_502`, `probe_fails_on_timeout` (asserting the Err string contains `timed out`), each using a local tokio::net::TcpListener and an http:// URL.",
        "Add `HealthCheckSettings { pub enabled: bool, pub failover: bool }` to crates/core/src/models/settings.rs next to RealDelaySettings (anchor 'impl Default for RealDelaySettings {'), deriving Debug, Clone, PartialEq, Eq, Serialize, Deserialize, with a manual `impl Default` setting enabled = true, failover = false.",
        "Add `#[serde(default)] pub health_check: HealthCheckSettings,` immediately after the anchor '    pub real_delay: RealDelaySettings,' in AppSettings, and `health_check: HealthCheckSettings::default(),` after the anchor '            real_delay: RealDelaySettings::default(),' in impl Default.",
        "Add `HealthCheckSettings` to the explicit re-export list at anchor 'pub use settings::{' in crates/core/src/models/mod.rs.",
        "Implement `pub async fn probe_via_http_proxy(proxy: SocketAddr, url: &str, timeout: Duration) -> Result<(), String>` in crates/subscription/src/health.rs: build a per-call client with `reqwest::Client::builder().proxy(reqwest::Proxy::all(format!(\"http://{proxy}\")).map_err(|e| e.to_string())?).timeout(timeout).build()`, GET the url, return Ok(()) when `response.status().as_u16() < 400`, otherwise `Err(status.to_string())`; on a transport error return `Err(\"timed out\".to_string())` when `err.is_timeout()` and `Err(err.to_string())` otherwise.",
        "Register the module: add `pub(crate) mod health;` after the anchor 'pub(crate) mod fetch;' in crates/subscription/src/lib.rs, and `pub use health::probe_via_http_proxy;` beside the anchor 'pub use real_delay::{RealDelayReport, measure_real_delay};'."
      ]
    },
    {
      "id": "ui-health-trackers",
      "tasks": [
        "2.1",
        "2.2"
      ],
      "summary": "NO-RED-WAIVER: no Rust red-stage agent; the coder writes the assertions first in its own chunk. NO-TESTER-WAIVER: same. Two pure state machines in one new ui module, both clock-injected so every assertion is deterministic. Shape follows crates/ui/src/failure_streak.rs, which already reports an edge once per streak. Test files the coder may touch: the inline `mod tests` in the new crates/ui/src/health.rs only.",
      "contract": {
        "states": [
          "healthy",
          "failing_1",
          "failing_2",
          "unhealthy",
          "dns_clear",
          "dns_failing"
        ],
        "transitions": [
          {
            "input": "record(Err(reason)) with 0 consecutive failures recorded",
            "state": "failing_1",
            "effect": "no-op",
            "evidence": "requirement: 'After 3 consecutive failures the session SHALL be unhealthy'"
          },
          {
            "input": "record(Err(reason)) with 1 consecutive failure recorded",
            "state": "failing_2",
            "effect": "no-op",
            "evidence": "requirement: 'After 3 consecutive failures the session SHALL be unhealthy'"
          },
          {
            "input": "record(Err(reason)) with 2 consecutive failures recorded",
            "state": "unhealthy",
            "effect": "set",
            "evidence": "requirement: 'After 3 consecutive failures the session SHALL be unhealthy' — returns Some(Health::Unhealthy(reason))"
          },
          {
            "input": "record(Err(reason)) while already unhealthy",
            "state": "unhealthy",
            "effect": "no-op",
            "evidence": "requirement scenario 'Continued failures stay quiet'"
          },
          {
            "input": "record(Ok(())) while unhealthy",
            "state": "healthy",
            "effect": "set",
            "evidence": "requirement: 'the next success SHALL make it healthy' — returns Some(Health::Healthy)"
          },
          {
            "input": "record(Ok(())) while last_reported is None (fresh tracker or after reset)",
            "state": "healthy",
            "effect": "set",
            "evidence": "connection-auto-resolve requirement: 'a successful probe ... SHALL reset that count' — without this edge the failover budget can never be reset by a probe; returns Some(Health::Healthy)"
          },
          {
            "input": "record(Ok(())) while last_reported is Some(Health::Healthy)",
            "state": "healthy",
            "effect": "no-op",
            "evidence": "requirement scenario 'Isolated failure': one failure then a success leaves the session healthy and reports nothing new"
          },
          {
            "input": "record(Err(_)) after a single earlier failure followed by record(Ok(()))",
            "state": "failing_1",
            "effect": "forced",
            "evidence": "requirement: 'After 3 consecutive failures' — the counter is reset by any success"
          },
          {
            "input": "reset()",
            "state": "healthy",
            "effect": "forced",
            "evidence": "requirement: 'Probing SHALL pause while the backend is being respawned and restart from a healthy state' — clears the counter and last_reported, returns nothing"
          },
          {
            "input": "observe(line containing 'app/dns: failed to retrieve response', now)",
            "state": "dns_clear",
            "effect": "set",
            "evidence": "requirement: 'the system SHALL count backend log lines reporting `app/dns: failed to retrieve response`' — records the timestamp"
          },
          {
            "input": "observe(line containing 'proxy/tun: connection reset by peer', now)",
            "state": "dns_clear",
            "effect": "no-op",
            "evidence": "requirement: 'Log lines that carry no destination, such as `proxy/tun: connection reset by peer` and `proxy/tun: connection was refused`, SHALL NOT affect health'"
          },
          {
            "input": "is_failing(now) with 20 or more counted lines whose timestamps are within 60 s of now",
            "state": "dns_failing",
            "effect": "set",
            "evidence": "requirement: 'SHALL mark DNS through the proxy as failing when at least 20 such lines occur within 60 seconds'"
          },
          {
            "input": "is_failing(now) 60 s after the newest counted line",
            "state": "dns_clear",
            "effect": "forced",
            "evidence": "requirement: 'the mark SHALL clear after 60 seconds without such a line'"
          }
        ],
        "forbidden": [
          "reset() returning a transition — the App has already cleared health on the non-Running state that caused the reset, so a Healthy emission there would repaint a Starting status bar",
          "DnsFailureWindow reading Instant::now() internally — `now` is a parameter on both observe and is_failing, so tests drive synthetic time",
          "DnsFailureWindow counting any line whose text starts with 'proxy/tun:'",
          "Health without a Debug derive — AppMsg is declared '#[derive(Debug)]' at anchor 'pub enum AppMsg {' and will not compile otherwise",
          "HealthTracker holding a reqwest client, a SocketAddr or any IO — it records outcomes only"
        ],
        "seeding": [
          "healthy / failing_1 / failing_2 / unhealthy: only `HealthTracker::new()` followed by `record(Ok(()))` / `record(Err(String))` calls. Never by writing the internal counter.",
          "the after-reset healthy state: only `HealthTracker::reset()`.",
          "dns_clear / dns_failing: only `DnsFailureWindow::default()` then `observe(line, now)` with Instants built as `base + Duration::from_secs(k)` from one `Instant::now()` captured at the top of the test."
        ],
        "budgets": [
          "FAILURE_THRESHOLD = 3 (consecutive probe failures before Unhealthy)",
          "DNS_FAILURE_THRESHOLD = 20 (counted lines)",
          "DNS_WINDOW = Duration::from_secs(60)",
          "noise test feeds 500 'proxy/tun: connection reset by peer' lines and asserts is_failing stays false",
          "DNS_RECHECK_INTERVAL: Duration::from_secs(10) — the timer tick that re-evaluates DnsFailureWindow::is_failing so the 60 s clear edge fires without a further log line"
        ]
      },
      "codeTasks": [
        "TEST FIRST: create the `mod tests` in crates/ui/src/health.rs with `two_failures_report_nothing_third_reports_unhealthy`, `further_failures_stay_quiet`, `success_after_unhealthy_reports_healthy`, `first_success_of_a_session_reports_healthy`, `success_between_failures_resets_the_streak`, `reset_clears_the_streak_and_reports_nothing`, `dns_window_marks_failing_at_twenty_within_sixty_seconds`, `dns_window_clears_sixty_seconds_after_the_last_line`, `dns_window_ignores_tun_noise` (500 lines).",
        "Create crates/ui/src/health.rs with `#[derive(Debug, Clone, PartialEq, Eq)] pub(crate) enum Health { Healthy, Unhealthy(String) }`, `const FAILURE_THRESHOLD: u32 = 3;`, and `pub(crate) struct HealthTracker { failures: u32, last_reported: Option<Health> }` exposing `new()`, `record(&mut self, outcome: Result<(), String>) -> Option<Health>` and `reset(&mut self)` per the transition table.",
        "Add `pub(crate) struct DnsFailureWindow` to the same file: `observe(&mut self, line: &str, now: Instant)` pushes `now` only when `line.contains(\"app/dns: failed to retrieve response\")` and drops entries older than DNS_WINDOW; `is_failing(&self, now: Instant) -> bool` counts entries within DNS_WINDOW of `now` and compares against DNS_FAILURE_THRESHOLD. Constants `DNS_FAILURE_THRESHOLD: usize = 20` and `DNS_WINDOW: Duration = Duration::from_secs(60)` live in this file.",
        "Register the module: add `pub(crate) mod health;` after the anchor 'pub(crate) mod failure_streak;' in crates/ui/src/lib.rs."
      ]
    },
    {
      "id": "preferences-page",
      "tasks": [
        "4.4"
      ],
      "summary": "NO-RED-WAIVER: no Rust red-stage agent. NO-TESTER-WAIVER: same — and this seam has no unit-testable surface: build_network_page needs a live GTK/adwaita instance, so its proof is the manual check in the verification seam plus compilation. Two switch rows bound to the fields seam settings-and-probe adds. Depends on settings-and-probe being merged. Test files the coder may touch: none.",
      "contract": {
        "states": [
          "row_reflects_settings",
          "settings_updated",
          "restart_banner_quiet"
        ],
        "transitions": [
          {
            "input": "page built with health_check.enabled = true",
            "state": "row_reflects_settings",
            "effect": "set",
            "evidence": "anchor '    let real_delay_use_for_lowest_row = adw::SwitchRow::builder()' — .active(s.real_delay.use_for_lowest_latency) is the exact binding shape"
          },
          {
            "input": "user toggles 'Check connection health'",
            "state": "settings_updated",
            "effect": "set",
            "evidence": "anchor '        real_delay_use_for_lowest_row.connect_active_notify(move |row| {' — st.borrow_mut() mutation followed by emit(&st, &cb)"
          },
          {
            "input": "user toggles 'Reconnect when the proxy stops responding'",
            "state": "settings_updated",
            "effect": "set",
            "evidence": "same anchor, copied a second time"
          },
          {
            "input": "either toggle while a connection is Running",
            "state": "restart_banner_quiet",
            "effect": "no-op",
            "evidence": "anchor 'snapshot.diverges_from(&self.settings, &current_rules, manual_nodes, subscriptions)' — only RuntimeConfigSnapshot fields can raise the banner, and health_check is deliberately not one of them"
          }
        ],
        "forbidden": [
          "adding health_check to RuntimeConfigSnapshot (anchor 'self.runtime_snapshot = Some(RuntimeConfigSnapshot {') — it would make each toggle raise the restart banner",
          "building the rows after the anchor '    drop(s);' — `s` is the borrow the .active(...) reads come from"
        ],
        "seeding": [
          "row_reflects_settings and settings_updated are reachable only through build_network_page with a live adw::PreferencesPage; there is no headless path, so both are proven by the manual step in the verification seam."
        ],
        "budgets": [
          "two rows, one PreferencesGroup, added after the group built at anchor '    let real_delay_group = adw::PreferencesGroup::builder()'",
          "zero new unit tests in this seam"
        ]
      },
      "codeTasks": [
        "In crates/ui/src/preferences/network.rs, before the anchor '    drop(s);', add a `adw::PreferencesGroup` titled 'Connection health' holding `health_check_enabled_row` (adw::SwitchRow, title 'Check connection health', .active(s.health_check.enabled)) and `health_check_failover_row` (title 'Reconnect when the proxy stops responding', .active(s.health_check.failover)); `page.add(&health_check_group);`.",
        "After the anchor '        real_delay_use_for_lowest_row.connect_active_notify(move |row| {' block, add two identically shaped blocks: `st.borrow_mut().health_check.enabled = row.is_active(); emit(&st, &cb);` and the same for `.failover`.",
        "Do not touch crates/ui/src/app.rs in this seam."
      ]
    },
    {
      "id": "ui-wiring",
      "tasks": [
        "3.1",
        "3.2",
        "4.1",
        "4.3"
      ],
      "summary": "NO-RED-WAIVER: no Rust red-stage agent; the coder writes the connection-task and pure-helper assertions first inside this chunk. NO-TESTER-WAIVER: same. The load-bearing seam: the health monitor task in the candidate loop, the xray DNS window fed off the log forwarder, the two new AppMsg variants with their generation-guarded handlers, the status text, and the opt-in failover. It also pays the sprint's shared debt — one halt set covering every spawned task on every exit path. Test files the coder may touch: the inline `mod tests` in crates/ui/src/connection.rs and in crates/ui/src/app.rs. Merge after settings-and-probe and ui-health-trackers; before the tray-notify seam, which edits app.rs too.",
      "contract": {
        "states": [
          "monitor_absent",
          "monitor_probing",
          "monitor_paused",
          "monitor_halted",
          "app_health_none",
          "app_health_healthy",
          "app_health_unhealthy",
          "app_dns_failing",
          "failover_armed",
          "failover_blocked"
        ],
        "transitions": [
          {
            "input": "candidate reported Running with settings.health_check.enabled == true",
            "state": "monitor_probing",
            "effect": "set",
            "evidence": "anchor '                    report(ProcessState::Running, Some(meta.clone()));' — the monitor is spawned immediately after it, beside the two existing forwarders"
          },
          {
            "input": "candidate reported Running with settings.health_check.enabled == false",
            "state": "monitor_absent",
            "effect": "no-op",
            "evidence": "requirement: 'When health checks are disabled, no probe SHALL be sent.'"
          },
          {
            "input": "StateChanged { to: ProcessState::Starting } on the monitor's own receiver",
            "state": "monitor_paused",
            "effect": "forced",
            "evidence": "anchor 'let _ = self.state.transition(ProcessState::Starting, self.current_connection.clone());' in handle_unexpected_exit — the respawn relay; the monitor calls HealthTracker::reset() and stops probing"
          },
          {
            "input": "StateChanged { to: ProcessState::Running } on the monitor's own receiver",
            "state": "monitor_probing",
            "effect": "forced",
            "evidence": "requirement: 'Probing SHALL pause while the backend is being respawned and restart from a healthy state when it is `Running` again.'"
          },
          {
            "input": "broadcast::error::RecvError::Lagged on the monitor's receiver",
            "state": "monitor_probing",
            "effect": "no-op",
            "evidence": "anchor 'Err(broadcast::error::RecvError::Lagged(_)) => continue,' — the existing forwarders' handling; a busy log burst must not kill the monitor"
          },
          {
            "input": "ConnectionCmd::Stop arm of the supervision loop",
            "state": "monitor_halted",
            "effect": "forced",
            "evidence": "anchor '                        halt(state_forwarder).await;' inside the Some(ConnectionCmd::Stop) arm — today this path leaks log_forwarder; the replacement halts every spawned task"
          },
          {
            "input": "the '_ =>' arm after mgr.wait_and_handle_exit()",
            "state": "monitor_halted",
            "effect": "forced",
            "evidence": "anchor '                                halt(state_forwarder).await;' inside the `_ =>` arm — the second leaking path"
          },
          {
            "input": "supervision loop breaks on ProcessState::Error (candidate give-up)",
            "state": "monitor_halted",
            "effect": "forced",
            "evidence": "anchor '            halt(log_forwarder).await;' — the only path that halts both today"
          },
          {
            "input": "HealthTracker::record returns Some(health)",
            "state": "app_health_unhealthy or app_health_healthy",
            "effect": "set",
            "evidence": "requirement: 'After 3 consecutive failures the session SHALL be unhealthy' — the monitor emits AppMsg::ConnectionHealth(generation, health)"
          },
          {
            "input": "log line while settings.backend.backend_type == BackendType::Xray and DnsFailureWindow::is_failing flips",
            "state": "app_dns_failing",
            "effect": "set",
            "evidence": "anchor '                        Ok(ProcessEvent::LogLine(line)) => {' — emits AppMsg::DnsHealth(generation, bool) beside the existing AppMsg::ProcessLogLine"
          },
          {
            "input": "the same log lines while backend_type is SingBox or V2ray",
            "state": "app_dns_failing",
            "effect": "no-op",
            "evidence": "requirement: 'Other backends SHALL NOT be classified from their log output.'"
          },
          {
            "input": "AppMsg::ConnectionHealth or AppMsg::DnsHealth carrying a superseded generation",
            "state": "app_health_none",
            "effect": "no-op",
            "evidence": "anchor '                if !is_current_generation(generation, self.connection_generation) {' in the AppMsg::ProcessLogLine arm — the same guard"
          },
          {
            "input": "App::apply_state with any state other than ProcessState::Running",
            "state": "app_health_none",
            "effect": "clear",
            "evidence": "anchor '        self.process_state = state.clone();' — health and dns_failing are cleared here so a stale Unhealthy can never colour a Starting"
          },
          {
            "input": "status_texts with Running, metadata present, health Unhealthy(reason)",
            "state": "app_health_unhealthy",
            "effect": "set",
            "evidence": "requirement: 'An unhealthy session SHALL show `Proxy not responding` as the status text, with the connection details followed by the last probe failure.'"
          },
          {
            "input": "status_texts with Running, metadata present, health Healthy, dns_failing true",
            "state": "app_dns_failing",
            "effect": "set",
            "evidence": "requirement: 'A healthy session whose DNS through the proxy is marked failing SHALL show `Connected` with the details prefixed by `DNS via proxy failing`.'"
          },
          {
            "input": "status_texts with Running, metadata present, health Healthy, dns_failing false",
            "state": "app_health_healthy",
            "effect": "no-op",
            "evidence": "requirement: 'A healthy session without the DNS mark SHALL show the existing connected text.' — anchor '                (\"Connected\".to_string(), details)'"
          },
          {
            "input": "status_texts with Starting while a stale Unhealthy is still in the view struct",
            "state": "app_health_none",
            "effect": "no-op",
            "evidence": "sprint arm table: health is read only in the Running arms — anchor '            (ProcessState::Starting, _) => (\"Connecting…\".to_string(), \"Resolving nodes\".into()),'"
          },
          {
            "input": "Unhealthy transition while health_check.failover is true, session_target is None and health_failovers < MAX_AUTO_RECONNECTS",
            "state": "failover_armed",
            "effect": "set",
            "evidence": "requirement: 'When it is on and a session started by the configured strategy becomes unhealthy, the system SHALL disconnect and reconnect using the configured strategy with the unhealthy node excluded from that attempt's candidates.'"
          },
          {
            "input": "Unhealthy transition while health_check.failover is false",
            "state": "failover_blocked",
            "effect": "no-op",
            "evidence": "requirement scenario 'Failover off by default'"
          },
          {
            "input": "Unhealthy transition while session_target is Some(_)",
            "state": "failover_blocked",
            "effect": "no-op",
            "evidence": "requirement: 'A session started by a direct connection to a chosen node SHALL NOT fail over for health, regardless of the preference.'"
          },
          {
            "input": "Unhealthy transition while health_failovers == MAX_AUTO_RECONNECTS",
            "state": "failover_blocked",
            "effect": "no-op",
            "evidence": "requirement: 'The system SHALL perform at most 3 consecutive health failovers without a successful probe in between' — anchor 'const MAX_AUTO_RECONNECTS: u32 = 3;'"
          },
          {
            "input": "terminal Stopped arriving while health_reconnect_pending is true",
            "state": "failover_armed",
            "effect": "forced",
            "evidence": "requirement: 'the system SHALL disconnect and reconnect using the configured strategy' — dispatches AppMsg::Connect(ConnectOrigin::AutoReconnect) directly, not through the reconnect_pending path at anchor '                    sender.input(reconnect_msg(self.session_target, ConnectOrigin::Restart));' which would force ConnectOrigin::Restart"
          },
          {
            "input": "AppMsg::Connect handler after planner.plan",
            "state": "failover_armed",
            "effect": "clear",
            "evidence": "anchor '                let candidates = planner.plan(&subscriptions, &manual_nodes);' — self.excluded_node.take() removes the node for this attempt only"
          },
          {
            "input": "AppMsg::Connect or AppMsg::ConnectToNode with cancels_auto_reconnect(origin) true, or AppMsg::Disconnect",
            "state": "failover_blocked",
            "effect": "clear",
            "evidence": "requirement: 'a user-initiated Connect, direct connect, or Disconnect SHALL reset that count' — anchor 'fn cancels_auto_reconnect(origin: ConnectOrigin) -> bool {'"
          },
          {
            "input": "AppMsg::ConnectionHealth carrying Health::Healthy for the current generation",
            "state": "failover_blocked",
            "effect": "clear",
            "evidence": "requirement: 'a successful probe ... SHALL reset that count' — health_failovers = 0"
          },
          {
            "input": "AppMsg::Disconnect issued by the health failover (health_reconnect_pending true)",
            "state": "failover_armed",
            "effect": "no-op",
            "evidence": "crates/ui/src/app.rs Disconnect arm calls cancel_auto_reconnect() on every Disconnect; the self-issued stop must keep excluded_node and health_failovers"
          },
          {
            "input": "recheck tick with no counted line in the last 60 s",
            "state": "dns_clear",
            "effect": "set",
            "evidence": "emits AppMsg::DnsHealth(generation, false); DnsFailureWindow::is_failing(now) is false once DNS_WINDOW has elapsed since the newest counted line"
          }
        ],
        "forbidden": [
          "resetting health_failovers inside cancel_auto_reconnect (anchor '    fn cancel_auto_reconnect(&mut self) {') — it runs on every ProcessState::Running at anchor '                            self.cancel_auto_reconnect();', which would refill the budget after each failover and allow an unbounded loop",
          "reusing auto_reconnect_attempts (anchor '    auto_reconnect_attempts: u32,') as the health failover counter, for the same reason",
          "routing the health failover through self.reconnect_pending — the path at anchor '                    sender.input(reconnect_msg(self.session_target, ConnectOrigin::Restart));' hardcodes ConnectOrigin::Restart, and cancels_auto_reconnect(Restart) is true, so the Connect arm would clear excluded_node and reset health_failovers before the plan is filtered",
          "reading self.connection_status for the excluded node after dispatching AppMsg::Disconnect — the node_ref must be captured in the ConnectionHealth handler first",
          "spawning the monitor with the same broadcast receiver the state_forwarder holds — it takes a second mgr.subscribe(); the forwarder at anchor '            let mut state_rx = mgr.subscribe();' stays untouched",
          "returning from any post-Running exit path of the supervision loop without halting the monitor — a surviving monitor keeps connecting to the proxy port after handle.stop(), which this seam's own test asserts against",
          "probing at all when settings.health_check.enabled is false",
          "adding health_check to RuntimeConfigSnapshot",
          "hardcoding 10 s / 30 s inside the monitor body with no test-visible override — the harness cannot wait 10 s inside RECV_TIMEOUT = 20 s for a three-failure streak"
        ],
        "seeding": [
          "ProcessState::Running for a connection: only `connect(&stub, settings, vec![candidate(addr)])` (or the new connect_with_health) followed by `next_state(&rx)`. Never by constructing a ProcessState or emitting AppMsg by hand.",
          "monitor_paused: only a stub script that exits on its own so ProcessManager::handle_unexpected_exit respawns it (the existing crash-respawn stub pattern in crates/ui/src/connection.rs tests). Never by publishing a StateChanged event directly.",
          "monitor_halted: only `handle.stop()` followed by `assert_nothing_after_terminal(&rx)` plus an assertion that the fake proxy listener accepted no further connection.",
          "the probe endpoint: bind a tokio::net::TcpListener on 127.0.0.1:0, read its port, and pass that port as `http` to `v2ray_settings(socks, http, \"127.0.0.1\", false)`; the listener counts accepts and switches its canned reply from 204 to 502 on command.",
          "app_health_* and failover_* states: assert on the pure helpers only — `status_texts`, `should_announce`, `health_failover_allowed`, `exclude_candidate`. crates/ui/src/app.rs has no headless harness for App::update (its tests build fixtures like `snapshot(backend, tun_enabled)` and `session_target_node()` and call free functions), so no test constructs an App or drives a message through it.",
          "the xray DNS path: a stub script that prints 25 `app/dns: failed to retrieve response` lines, run with a settings value whose backend_type is Xray, drained with `drain(&rx)`; the sing-box counterpart uses `singbox_settings()` with the same script."
        ],
        "budgets": [
          "HEALTH_INITIAL_DELAY = Duration::from_secs(10)",
          "HEALTH_INTERVAL = Duration::from_secs(30)",
          "per-probe timeout = Duration::from_millis(u64::from(settings.real_delay.timeout_ms)) — the field is u32 milliseconds",
          "FAILURE_THRESHOLD = 3 consecutive failures before AppMsg::ConnectionHealth(_, Health::Unhealthy(_))",
          "MAX_AUTO_RECONNECTS = 3 health failovers",
          "test timing override: HealthTiming { initial_delay: Duration::from_millis(50), interval: Duration::from_millis(100) } so a three-failure streak lands well inside RECV_TIMEOUT = 20 s",
          "existing connection tests keep HealthTiming::default(), whose 10 s first probe never fires inside their lifetime",
          "DNS_RECHECK_INTERVAL: Duration::from_secs(10) — the timer tick that re-evaluates DnsFailureWindow::is_failing so the 60 s clear edge fires without a further log line"
        ]
      },
      "codeTasks": [
        "TEST FIRST in crates/ui/src/connection.rs `mod tests`: `health_monitor_reports_unhealthy_then_healthy` (fake proxy listener; probes start failing -> exactly one AppMsg::ConnectionHealth(GENERATION, Health::Unhealthy(_)); restored -> exactly one Health::Healthy), `health_monitor_is_not_spawned_when_disabled` (health_check.enabled = false -> listener records zero accepts), `health_monitor_stops_with_the_connection` (handle.stop(); assert_nothing_after_terminal plus zero further accepts on the listener), `xray_dns_burst_reports_dns_health` (stub prints 25 matching lines -> exactly one AppMsg::DnsHealth(GENERATION, true)), `singbox_dns_burst_reports_nothing` (same stub output under singbox_settings() -> no AppMsg::DnsHealth).",
        "Add `HealthTiming { pub initial_delay: Duration, pub interval: Duration }` to crates/ui/src/connection.rs with a Default of 10 s / 30 s, add `pub health_timing: HealthTiming` to ConnectionRequest, destructure it in spawn_with beside `host_has_ipv6`, set `HealthTiming::default()` at the app.rs construction site, and add a `connect_with_health` test helper that overrides it.",
        "Replace the ad-hoc halts in the supervision loop with one helper that halts every spawned task (state_forwarder, log_forwarder, and the monitor when present) and call it on all three post-Running exit paths: the Some(ConnectionCmd::Stop) arm, the `_ =>` arm after mgr.wait_and_handle_exit(), and the fall-through after the loop breaks.",
        "Spawn the health monitor after the anchor '                    report(ProcessState::Running, Some(meta.clone()));' when effective_settings.health_check.enabled: it takes a second `mgr.subscribe()`, sleeps HealthTiming::initial_delay, then loops on a tokio::select! over the state receiver and an interval tick; Starting pauses probing and calls HealthTracker::reset(), Running resumes, Lagged continues, Closed breaks. Each tick calls v2ray_rs_subscription::probe_via_http_proxy(addr, &settings.real_delay.test_url, timeout) and emits AppMsg::ConnectionHealth(generation, health) for every Some(_) the tracker returns.",
        "Compute the probe address from the shared helper: `effective_settings.local_endpoint(effective_settings.http_port)` when confirm-backend-ready-before-running is already merged; otherwise add that helper to crates/core/src/models/settings.rs beside the anchor '    pub fn validate_listen_address(addr: &str) -> Result<(), ValidationError> {' with its four unit cases (127.0.0.1, 0.0.0.0 -> 127.0.0.1, :: -> ::1, 192.168.1.10).",
        "Feed the DNS window from the log forwarder: move the Copy backend_type into the task at the anchor '            let log_sender = sender.clone();', and inside the arm at anchor '                        Ok(ProcessEvent::LogLine(line)) => {' call DnsFailureWindow::observe when the backend is BackendType::Xray, emitting AppMsg::DnsHealth(generation, flag) only when is_failing flips.",
        "Add `ConnectionHealth(u64, Health)` and `DnsHealth(u64, bool)` to AppMsg after the anchor '    ProcessLogLine(u64, String),'.",
        "Add the two `AppMsg::ConnectionHealth` / `AppMsg::DnsHealth` match arms in the SAME chunk that adds the variants — `App::update`'s match at crates/ui/src/app.rs has no wildcard arm, so adding a variant without its arm is E0004 and h4's own `make test-ui` fails. h4 lands them behind the existing `is_current_generation` guard storing the value; h5 fills in the announce/failover behavior.",
        "Drive the DNS clear edge from a timer, not from log lines: the window is only evaluated inside the LogLine arm, so once a burst stops no further line arrives, `is_failing` is never re-evaluated and `DnsHealth(_, false)` is never emitted — the 60 s clear requirement would be unimplemented. Re-evaluate on a `tokio::time::interval(DNS_RECHECK_INTERVAL)` tick in the forwarder select (or on the health monitor's own interval tick).",
        "TEST FIRST: add `dns_window_clears_after_the_recheck_tick` — DnsFailureWindow takes `now` as a parameter, so drive 20 matching lines then a tick 61 s later and assert is_failing flips false; at the forwarder level ride the existing xray burst test with an injected short recheck interval and assert exactly one `AppMsg::DnsHealth(GENERATION, false)`",
        "TEST FIRST in crates/ui/src/app.rs `mod tests`: `status_texts_unhealthy_says_proxy_not_responding`, `status_texts_dns_failing_prefixes_details`, `status_texts_healthy_matches_todays_connected_text`, `status_texts_starting_ignores_stale_health`, `health_failover_allowed_rejects_direct_target`, `health_failover_allowed_rejects_exhausted_budget`, `health_failover_allowed_rejects_disabled_preference`, `exclude_candidate_drops_only_the_named_node`.",
        "Add `health: Option<Health>`, `dns_failing: bool`, `health_failovers: u32`, `excluded_node: Option<ConnectionNodeRef>` and `health_reconnect_pending: bool` to the App struct and to the model literal at the anchor '            session_target: None,'.",
        "Extract the body of `fn update_status_labels(&self)` into a pure function over the sprint's StatusView struct, adding the `health` and `dns_failing` fields to it (the 4.1 site states the fallback if StatusView is absent), and implement the three Running arms in the sprint's precedence order.",
        "Clear health and dns_failing in App::apply_state whenever the new state is not ProcessState::Running, at the anchor '        self.process_state = state.clone();'.",
        "Extend the two message handlers h4 created beside the anchor '            AppMsg::ProcessLogLine(generation, line) => {', both behind the existing is_current_generation guard; the ConnectionHealth handler stores the health, resets health_failovers on Healthy, calls update_status_labels, and on an Unhealthy transition captures self.connection_status.as_ref().map(|m| m.node_ref) BEFORE deciding on failover.",
        "Add `fn health_failover_allowed(enabled: bool, session_target: Option<SessionTarget>, count: u32) -> bool` and `fn exclude_candidate(candidates: Vec<ConnectionCandidate>, excluded: Option<ConnectionNodeRef>) -> Vec<ConnectionCandidate>` beside the anchor 'struct SessionTarget {'.",
        "Wire the failover: on an armed Unhealthy set excluded_node, increment health_failovers, set health_reconnect_pending and dispatch AppMsg::Disconnect; in the terminal branch of AppMsg::ProcessStateConnection check health_reconnect_pending BEFORE the reconnect_after_stop check and dispatch AppMsg::Connect(ConnectOrigin::AutoReconnect); apply exclude_candidate after the anchor '                let candidates = planner.plan(&subscriptions, &manual_nodes);'; reset health_failovers and clear excluded_node in the Connect/ConnectToNode arms when cancels_auto_reconnect(origin) is true and in the Disconnect arm ONLY when `!self.health_reconnect_pending` (the failover issues its own Disconnect and must keep its exclusion and budget).",
        "Gate the Disconnect-arm reset on `!self.health_reconnect_pending`: the failover dispatches AppMsg::Disconnect itself, and the existing arm clears `excluded_node` and `health_failovers` (and calls cancel_auto_reconnect) on every Disconnect — so without the carve-out the exclusion is cleared before AppMsg::Connect filters the plan, the budget never accumulates, and the same dead node is re-picked without bound."
      ]
    },
    {
      "id": "tray-notify",
      "tasks": [
        "4.2"
      ],
      "summary": "NO-RED-WAIVER: no Rust red-stage agent. NO-TESTER-WAIVER: same. Announce an unhealthy session once per streak: an in-window toast always, a desktop notification when notifications_enabled. The tray crate gains a public notify entry point; the ui crate gains the pure should_announce edge test and the lock-and-call free function. Merge AFTER the ui-wiring seam — both edit crates/ui/src/app.rs. Test files the coder may touch: the inline `mod tests` in crates/ui/src/app.rs; crates/tray has no unit-testable surface here (Notification::show needs a live notification daemon).",
      "contract": {
        "states": [
          "announced",
          "quiet"
        ],
        "transitions": [
          {
            "input": "should_announce(None, &Health::Unhealthy(reason))",
            "state": "announced",
            "effect": "set",
            "evidence": "requirement: 'When a session becomes unhealthy, the system SHALL show a notification in the main window'"
          },
          {
            "input": "should_announce(Some(&Health::Healthy), &Health::Unhealthy(reason))",
            "state": "announced",
            "effect": "set",
            "evidence": "requirement scenario 'First transition notifies'"
          },
          {
            "input": "should_announce(Some(&Health::Unhealthy(_)), &Health::Unhealthy(_))",
            "state": "quiet",
            "effect": "no-op",
            "evidence": "requirement: 'The system SHALL NOT repeat either while the session stays unhealthy.'"
          },
          {
            "input": "should_announce(_, &Health::Healthy)",
            "state": "quiet",
            "effect": "no-op",
            "evidence": "requirement names only the unhealthy transition as announced"
          },
          {
            "input": "an announced transition while settings.notifications_enabled is false",
            "state": "announced",
            "effect": "set",
            "evidence": "requirement: 'and, when desktop notifications are enabled, SHALL send a desktop notification' — the toast still shows, the desktop notification does not"
          }
        ],
        "forbidden": [
          "calling Notification::show on the GTK main thread — anchor '    fn send(&self, summary: &str, body: &str) {' is blocking; the tray service wraps its calls in tokio::task::spawn_blocking and the new path must do the same",
          "dropping the enabled gate — Notifier owns it (anchor '        if !self.enabled.load(Ordering::Relaxed) {' in on_state_change); the new public wrapper keeps the check inside the Notifier rather than at the call site",
          "holding the TRAY_HANDLE mutex across the blocking show call",
          "announcing from anywhere but the ConnectionHealth handler's transition edge"
        ],
        "seeding": [
          "announced / quiet: only by calling the pure `should_announce(prev, next)` with Health values built directly. No test drives a real notification: the desktop side needs a live org.freedesktop.Notifications service and is covered by the manual verification step."
        ],
        "budgets": [
          "at most one toast and one desktop notification per Healthy-or-None -> Unhealthy edge",
          "NOTIFICATION_TIMEOUT_MS = 5000, unchanged",
          "one new unit test in this seam"
        ]
      },
      "codeTasks": [
        "TEST FIRST in crates/ui/src/app.rs `mod tests`: `should_announce_fires_once_per_unhealthy_streak` covering all four rows of the transition table.",
        "Add `pub fn notify(&self, summary: &str, body: &str)` to Notifier in crates/tray/src/notification.rs, checking `self.enabled` then delegating to the existing private `send`.",
        "Add `pub fn notify(&self, summary: &str, body: &str)` to TrayHandle beside the anchor '    pub fn set_notifications_enabled(&mut self, enabled: bool) {' in crates/tray/src/tray.rs, delegating to the Notifier.",
        "Add `fn should_announce(prev: Option<&Health>, next: &Health) -> bool` near the anchor 'fn active_nodes_available(subscriptions: &[Subscription], manual_nodes: &[ManualNode]) -> bool {'.",
        "Add a free function beside the anchor 'fn update_tray_notification_setting(enabled: bool) {' that clones what it needs out of the TRAY_HANDLE lock, releases the lock, and runs the notify call inside tokio::task::spawn_blocking.",
        "In the AppMsg::ConnectionHealth handler added by the ui-wiring seam, call show_toast with `Proxy not responding: <reason>` on an announced edge, and the new notify path additionally when settings.notifications_enabled."
      ]
    },
    {
      "id": "verification",
      "tasks": [
        "5.1",
        "5.2"
      ],
      "summary": "NO-RED-WAIVER: no Rust red-stage agent. NO-TESTER-WAIVER: same. The whole-change floor plus the one live step. 5.2 is MANUAL: it needs a real xray binary, a real subscription and root nft rules, none of which a test harness can provide; it is a verification step, not a blocker. Test files the coder may touch: none.",
      "contract": {
        "states": [
          "workspace_green",
          "live_unhealthy_observed",
          "live_recovered"
        ],
        "transitions": [
          {
            "input": "TEST_TIMEOUT=10m make test",
            "state": "workspace_green",
            "effect": "set",
            "evidence": "Makefile anchor 'TEST := timeout $(TEST_TIMEOUT) $(CARGO) test' with 'TEST_ARGS := -- --test-threads=$(TEST_THREADS)'"
          },
          {
            "input": "live: nft rule dropping the connected node's address while a session runs",
            "state": "live_unhealthy_observed",
            "effect": "set",
            "evidence": "task 5.2 — status shows `Proxy not responding` with exactly one toast within about 2 minutes"
          },
          {
            "input": "live: the nft rule removed",
            "state": "live_recovered",
            "effect": "forced",
            "evidence": "task 5.2 — status returns to `Connected` with no toast"
          }
        ],
        "forbidden": [
          "a bare `cargo test` without the timeout and --test-threads cap — the Makefile targets carry both",
          "treating 5.2's absence as a failure of the change; it is marked manual"
        ],
        "seeding": [
          "workspace_green: only the Makefile targets below.",
          "live_unhealthy_observed / live_recovered: only a real xray backend with a real subscription and a root-installed nft rule in a dedicated test table, removed afterwards. Not reachable from any test harness."
        ],
        "budgets": [
          "workspace test wall clock: 10 minutes",
          "test threads: 4",
          "live detection window: about 2 minutes (10 s initial delay + 3 intervals of 30 s + probe timeouts)",
          "expected toasts during the live outage: exactly 1"
        ]
      },
      "codeTasks": [
        "Run `TEST_TIMEOUT=10m make test` and `make lint` and fix what they report.",
        "Perform the live xray check described in task 5.2 and record the observed status text, toast count and, with failover on, the node the reconnect picked. Report it as a manual result."
      ]
    }
  ],
  "requirements": [
    {
      "shall": "### Requirement: Opt-in failover of an unhealthy session The system SHALL provide a preference, off by default, to fail over a session that becomes unhealthy. When it is on and a session started by the configured strategy becomes unhealthy, the system SHALL disconnect and reconnect using the configured strategy with the unhealthy node excluded from that attempt's candidates. The system SHALL perform at most 3 consecutive health failovers without a successful probe in between; a successful probe or a user-initiated Connect, direct connect, or Disconnect SHALL reset that count. A session started by a direct connection to a chosen node SHALL NOT fail over for health, regardless of the preference. A user-initiated Connect, direct connect, or Disconnect SHALL cancel a pending health failover.",
      "tests": [
        "crates/subscription/src/health.rs::tests::probe_succeeds_on_204",
        "crates/subscription/src/health.rs::tests::probe_fails_on_502",
        "crates/subscription/src/health.rs::tests::probe_fails_on_timeout"
      ]
    },
    {
      "shall": "- **THEN** the session SHALL keep running on the same node",
      "tests": [
        "MANUAL task 5.2: live xray check — block the node address, observe the status text, the single toast, the recovery, and with failover on the node the reconnect picked"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL reconnect with the configured strategy and node A SHALL NOT be a candidate for that attempt",
      "tests": [
        "crates/ui/src/app.rs::tests::health_failover_allowed_rejects_direct_target",
        "crates/ui/src/app.rs::tests::exclude_candidate_drops_only_the_named_node",
        "crates/ui/src/app.rs::tests::health_failover_allowed_rejects_exhausted_budget",
        "crates/ui/src/app.rs::tests::health_failover_allowed_rejects_disabled_preference"
      ]
    },
    {
      "shall": "- **THEN** the session SHALL keep running on node A and only the unhealthy status SHALL be shown",
      "tests": [
        "crates/ui/src/health.rs::tests::two_failures_report_nothing_third_reports_unhealthy",
        "crates/ui/src/health.rs::tests::further_failures_stay_quiet"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL NOT fail over again and SHALL keep the last session running with the unhealthy status",
      "tests": [
        "crates/ui/src/health.rs::tests::two_failures_report_nothing_third_reports_unhealthy",
        "crates/ui/src/health.rs::tests::further_failures_stay_quiet"
      ]
    },
    {
      "shall": "### Requirement: Running connection is probed through its own inbound While a connection is `Running` and health checks are enabled (the default), the system SHALL request the configured Real Delay test URL through the session's local HTTP inbound, first 10 seconds after the connection is reported `Running` and then every 30 seconds, each request bounded by the configured Real Delay timeout. A response with a status below 400 SHALL count as a success; a connection, TLS, or protocol error or a timeout SHALL count as a failure. After 3 consecutive failures the session SHALL be unhealthy; the next success SHALL make it healthy. Probing SHALL pause while the backend is being respawned and restart from a healthy state when it is `Running` again. Health SHALL NOT change the process state. When health checks are disabled, no probe SHALL be sent.",
      "tests": [
        "crates/subscription/src/health.rs::tests::probe_succeeds_on_204",
        "crates/subscription/src/health.rs::tests::probe_fails_on_502",
        "crates/subscription/src/health.rs::tests::probe_fails_on_timeout"
      ]
    },
    {
      "shall": "- **THEN** the session SHALL be unhealthy while its process state remains `Running`",
      "tests": [
        "crates/ui/src/health.rs::tests::two_failures_report_nothing_third_reports_unhealthy",
        "crates/ui/src/health.rs::tests::further_failures_stay_quiet"
      ]
    },
    {
      "shall": "- **THEN** the session SHALL be healthy",
      "tests": [
        "MANUAL task 5.2: live xray check — block the node address, observe the status text, the single toast, the recovery, and with failover on the node the reconnect picked"
      ]
    },
    {
      "shall": "- **THEN** the session SHALL stay healthy",
      "tests": [
        "MANUAL task 5.2: live xray check — block the node address, observe the status text, the single toast, the recovery, and with failover on the node the reconnect picked"
      ]
    },
    {
      "shall": "- **THEN** no probe SHALL be sent until the respawned backend is `Running`, and the session SHALL then be healthy until three new consecutive failures",
      "tests": [
        "crates/subscription/src/health.rs::tests::probe_succeeds_on_204",
        "crates/subscription/src/health.rs::tests::probe_fails_on_502",
        "crates/subscription/src/health.rs::tests::probe_fails_on_timeout"
      ]
    },
    {
      "shall": "- **THEN** no probe SHALL be sent and the session SHALL never be marked unhealthy",
      "tests": [
        "crates/subscription/src/health.rs::tests::probe_succeeds_on_204",
        "crates/subscription/src/health.rs::tests::probe_fails_on_502",
        "crates/subscription/src/health.rs::tests::probe_fails_on_timeout"
      ]
    },
    {
      "shall": "### Requirement: Unhealthy session is announced once per streak When a session becomes unhealthy, the system SHALL show a notification in the main window stating that the proxy is not responding and naming the last probe failure, and, when desktop notifications are enabled, SHALL send a desktop notification with the same content. The system SHALL NOT repeat either while the session stays unhealthy.",
      "tests": [
        "crates/subscription/src/health.rs::tests::probe_succeeds_on_204",
        "crates/subscription/src/health.rs::tests::probe_fails_on_502",
        "crates/subscription/src/health.rs::tests::probe_fails_on_timeout"
      ]
    },
    {
      "shall": "- **THEN** one in-window notification and one desktop notification SHALL be shown",
      "tests": [
        "crates/ui/src/app.rs::tests::should_announce_fires_once_per_unhealthy_streak"
      ]
    },
    {
      "shall": "- **THEN** no new notification SHALL be shown",
      "tests": [
        "crates/ui/src/app.rs::tests::should_announce_fires_once_per_unhealthy_streak"
      ]
    },
    {
      "shall": "### Requirement: xray DNS through the proxy is flagged when failing When the backend is xray, the system SHALL count backend log lines reporting `app/dns: failed to retrieve response` and SHALL mark DNS through the proxy as failing when at least 20 such lines occur within 60 seconds; the mark SHALL clear after 60 seconds without such a line. Log lines that carry no destination, such as `proxy/tun: connection reset by peer` and `proxy/tun: connection was refused`, SHALL NOT affect health. The DNS mark SHALL NOT make the session unhealthy. Other backends SHALL NOT be classified from their log output.",
      "tests": [
        "crates/ui/src/health.rs::tests::two_failures_report_nothing_third_reports_unhealthy",
        "crates/ui/src/health.rs::tests::further_failures_stay_quiet"
      ]
    },
    {
      "shall": "- **THEN** DNS through the proxy SHALL be marked failing",
      "tests": [
        "crates/ui/src/health.rs::tests::dns_window_marks_failing_at_twenty_within_sixty_seconds",
        "crates/ui/src/app.rs::tests::status_texts_dns_failing_prefixes_details",
        "crates/ui/src/health.rs::tests::dns_window_clears_sixty_seconds_after_the_last_line",
        "crates/ui/src/health.rs::tests::dns_window_ignores_tun_noise"
      ]
    },
    {
      "shall": "- **THEN** the session SHALL stay healthy and DNS SHALL NOT be marked failing",
      "tests": [
        "crates/ui/src/health.rs::tests::dns_window_marks_failing_at_twenty_within_sixty_seconds",
        "crates/ui/src/app.rs::tests::status_texts_dns_failing_prefixes_details",
        "crates/ui/src/health.rs::tests::dns_window_clears_sixty_seconds_after_the_last_line",
        "crates/ui/src/health.rs::tests::dns_window_ignores_tun_noise"
      ]
    },
    {
      "shall": "- **THEN** the mark SHALL clear",
      "tests": [
        "MANUAL task 5.2: live xray check — block the node address, observe the status text, the single toast, the recovery, and with failover on the node the reconnect picked"
      ]
    },
    {
      "shall": "### Requirement: Status bar shows connection health While the connection is `Running`, the status bar SHALL reflect connection health. An unhealthy session SHALL show `Proxy not responding` as the status text, with the connection details followed by the last probe failure. A healthy session whose DNS through the proxy is marked failing SHALL show `Connected` with the details prefixed by `DNS via proxy failing`. A healthy session without the DNS mark SHALL show the existing connected text. The connect button SHALL keep its connected appearance in all three cases.",
      "tests": [
        "crates/subscription/src/health.rs::tests::probe_succeeds_on_204",
        "crates/subscription/src/health.rs::tests::probe_fails_on_502",
        "crates/subscription/src/health.rs::tests::probe_fails_on_timeout"
      ]
    },
    {
      "shall": "- **THEN** the status text SHALL be `Proxy not responding` and the details SHALL end with the failure reason",
      "tests": [
        "crates/ui/src/app.rs::tests::status_texts_unhealthy_says_proxy_not_responding",
        "crates/ui/src/app.rs::tests::status_texts_healthy_matches_todays_connected_text",
        "crates/ui/src/app.rs::tests::status_texts_starting_ignores_stale_health"
      ]
    },
    {
      "shall": "- **THEN** the status text SHALL be `Connected` and the details SHALL start with `DNS via proxy failing`",
      "tests": [
        "crates/ui/src/health.rs::tests::dns_window_marks_failing_at_twenty_within_sixty_seconds",
        "crates/ui/src/app.rs::tests::status_texts_dns_failing_prefixes_details",
        "crates/ui/src/health.rs::tests::dns_window_clears_sixty_seconds_after_the_last_line",
        "crates/ui/src/health.rs::tests::dns_window_ignores_tun_noise"
      ]
    },
    {
      "shall": "- **THEN** the status bar SHALL show the same text as any connected session",
      "tests": [
        "crates/ui/src/app.rs::tests::status_texts_unhealthy_says_proxy_not_responding",
        "crates/ui/src/app.rs::tests::status_texts_healthy_matches_todays_connected_text",
        "crates/ui/src/app.rs::tests::status_texts_starting_ignores_stale_health"
      ]
    }
  ],
  "testHarness": [
    "Stub / stub(script) — crates/ui/src/connection.rs — temp dir + AppPaths::for_profile_in(AppProfile::Test, ...) + a 0o755 /bin/sh backend script; the stub-backend harness for every connection test",
    "request(stub, settings, candidates) — crates/ui/src/connection.rs — a full ConnectionRequest with ConfigWriter, pid/geodata paths, GENERATION = 7, host_has_ipv6 = true",
    "connect / connect_with — crates/ui/src/connection.rs — relm4::channel::<AppMsg>() + spawn_with(request, tx, configure) -> (ConnectionHandle, Receiver<AppMsg>)",
    "next_state(rx) — crates/ui/src/connection.rs — awaits the next ProcessStateConnection with RECV_TIMEOUT = 20 s, asserting the generation",
    "drain(rx) — crates/ui/src/connection.rs — drains until the channel closes, returning (terminal state, all log lines) — the harness for log-derived assertions",
    "assert_nothing_after_terminal(rx) — crates/ui/src/connection.rs — proves no message follows the terminal state; the pattern for 'handle.stop() leaves no probe traffic'",
    "candidate(address) / xhttp_candidate / node(address) — crates/ui/src/connection.rs — ConnectionCandidate with a Manual node_ref and a Shadowsocks (or VLESS/XHTTP) ProxyNode",
    "singbox_settings / tun_settings / v2ray_settings(socks, http, listen, tun) — crates/ui/src/connection.rs — AppSettings variants per backend; v2ray_settings is the one that sets listen_address + http_port, reusable for the probe address test",
    "capless_probe(mgr) — crates/ui/src/connection.rs — ProcessManager::with_host_probe(HostProbe { getcap: /bin/true, helper: /bin/true }) — makes TUN starts fail deterministically",
    "executable(dir, name) — crates/ui/src/connection.rs — a 0o755 `exit 0` script at dir/name",
    "attempts(marker) — crates/ui/src/connection.rs — counts lines a stub appended, i.e. how many candidate launches happened",
    "spawn_mock_clash(responses) — crates/subscription/src/real_delay.rs — a tokio TcpListener speaking raw HTTP/1.1 with canned status + body — the template for the fake HTTP proxy listener in 1.2 and 3.1",
    "install_crypto_provider() — crates/subscription/src/real_delay.rs — one-shot rustls ring default provider install; required before any reqwest https client in tests",
    "pick_free_loopback_port() — crates/subscription/src/real_delay.rs — binds 127.0.0.1:0, reads the port, drops the listener — the port helper for probe tests",
    "snapshot(backend, tun_enabled) / session_target_node() / routing_rule / profile_sub — crates/ui/src/app.rs — RuntimeConfigSnapshot, ConnectionNodeRef and routing/subscription fixtures for the pure-helper tests in app.rs's test module",
    "sample_metadata() — crates/tray/src/tray.rs — a ConnectionMetadata fixture for status-text tests"
  ],
  "floor": "TEST_TIMEOUT=10m make test && make lint   — `make test` expands to `timeout 10m cargo test --workspace --all-targets -- --test-threads=4` (Makefile anchors 'TEST := timeout $(TEST_TIMEOUT) $(CARGO) test' and 'TEST_ARGS := -- --test-threads=$(TEST_THREADS)'); `make lint` runs `cargo fmt -- --check` then `cargo clippy --workspace --all-targets --all-features -- -D warnings`. Plus the manual live step of task 5.2, which needs a real xray binary, a real subscription and root nft rules and is not runnable in a harness.",
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
