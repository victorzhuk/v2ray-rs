## Context

- `crates/ui/src/connection.rs:153-156`: inside the candidate loop, for every candidate, `pin_node_addresses` runs and `hosts_cover_nodes` decides `pinned`, regardless of `settings.tun.enabled`.
- `connection.rs:310-346`: `tokio::net::lookup_host` (OS resolver) per hostname, `PIN_LOOKUP_TIMEOUT` 5 s, sequential; failures only `log::warn!`. Hostnames that already have a user host override are skipped. Answers go into `dns.hosts` of the effective settings, so the generated config keeps them for the session, including respawns.
- `connection.rs:348-363`, `:367-396`: `capture_dns` for xray requires every node hostname to have a usable override; otherwise the helper runs without `--capture-dns`, silently.
- `connection.rs:111-113`, `:229`, `:301`: a failed candidate's manager is kept in `parked` with its routing state while the next candidate is prepared; for xray with `strict_route` that includes the table-2023 unreachable fallback and, with capture, the pref 8999 port-53 rules. Automatic reconnects also keep the strict state installed (tun-mode "Traffic stays blocked while an xray TUN session reconnects").
- Network facts measured in `archive/2026-09-03-fix-tun-dns-bootstrap/design.md`: DoH on 443 to public resolvers is blocked outside the tunnel; UDP/53 is answered by the network for any destination.
- All proxy transports (`TransportSettings`: Tcp, Ws, Grpc, H2, Xhttp) dial TCP to `address:port`, so a TCP connect is a valid reachability signal.
- `crates/process/src/manager.rs:395-415` writes the session record `backend=… version=… node=… tun=…`.

## Goals / Non-Goals

**Goals:**
- No pin lookup or probe runs through a tunnel this app installed.
- A poisoned or dead address in the pin set does not become the only address the backend dials.
- Disabled DNS capture is visible.

**Non-Goals:**
- Re-resolving during a running session or across respawns (the frozen set is what keeps the backend off the tunnel's resolver).
- Changing the generators' bootstrap DNS servers (`fix-tun-dns-bootstrap`).
- sing-box `auto_route` DNS behavior; sing-box pins are filtered the same way but capture is xray-only.

## Decisions

- **Pin only when TUN is active for the candidate's effective settings** (`tun.enabled` and backend not v2ray). Without a tunnel the backend's own dial-time lookup cannot loop, and a frozen answer only adds staleness. Alternative rejected: keep pinning everywhere for consistency — it has no benefit and carries the poisoning risk into non-TUN sessions.
- **Resolve once per connection attempt, before the first candidate.** Collect the unique hostnames of every candidate node and of the nodes each candidate's effective rules route through (`resolve_effective_config` + `resolve_via_nodes`, as the loop does today), resolve concurrently (at most 16 in flight) under one 5 s deadline, and apply the results to each candidate's effective settings in the loop. The lifecycle lock is held, so no earlier connection of this app has routes installed at that point. Alternative rejected: resolve lazily per candidate but shut down the parked manager first — that drops the kill switch the parking exists for.
- **Keep the OS resolver.** Custom UDP/53 gets the same intercepted answer; direct DoH is blocked on the measured network; resolving through the proxy needs the node already resolved. The reachability probe is the mitigation for bad answers. Marked conservative: it keeps the only transport known to work before a tunnel exists.
- **TCP-probe pins right before each candidate's config is written.** Connect to every pinned address of the candidate's hostnames on the node port, concurrently, 1.5 s per connect, 2 s total per candidate; keep the addresses that connected. None connected → keep all and log a notice (a firewall that only lets the backend through, or a node that is down, must not be made worse). Skipped, keeping all, when a parked manager holds TUN routing state, because the probe would measure the kill switch. Alternative rejected: probe everything up front — attempts plan every enabled node (hundreds here), which means hundreds of connects and tens of seconds before the first start.
- **Last-good fallback.** When a candidate reaches `Running`, the connection task sends its applied pins to the app (`AppMsg` with the generation), which keeps them in memory for the process lifetime. Each `ConnectionRequest` carries them; a hostname whose lookup fails or times out uses its last-good addresses (still probed). This covers automatic reconnects under the strict kill switch, where unmarked lookups are unroutable. Not persisted: a stale file across restarts would outlive the network it was measured on.
- **Surface disabled capture once per attempt.** When backend is xray, TUN is on, `dns_hijack` is `Hijack`, and a candidate is started unpinned: a notice line through the process log stream (`notice: DNS capture off for this session: <host> has no usable pinned address`; a host whose answers are all of a family the backend will not use counts as unpinned too, so the wording does not claim the lookup failed), a toast via `AppMsg::ShowToast`, and the existing `capture_dns=false` field of that launch's session record states it (`capture_dns=true` when armed). Emitted once per attempt even across candidates.

## Risks / Trade-offs

- [All addresses of a host answer TCP but some are still wrong (a transparent proxy accepting anything)] → no worse than today; the probe only removes addresses that provably do not accept.
- [Probe connects to node servers from the host IP] → one connect per address per candidate start, the same destination the backend dials seconds later.
- [5 s up-front deadline with many unique hostnames] → concurrency 16 with typical sub-100 ms answers covers hundreds of names; unresolved names fall back to last-good or stay unpinned with the notice.
- [Existing users relying on pins without TUN] → none known; behavior returns to backend dial-time resolution, which was the pre-pinning behavior.

## Implementation plan

Tier `heavy`, mode `existing-service-strict`, lenses `spec`, `quality`, `perf` (`perf`: concurrent lookups and probes on the connect path). Planned at `8bc03f0e`.

Rust has no red-stage test-writer agent, so every seam carries `NO-RED-WAIVER` / `NO-TESTER-WAIVER` and its tests are the first `codeTasks` of the chunk that owns them.

Scope decisions taken while planning: the delta spec forbids lookups only while routing state of an earlier candidate of the same attempt is installed (an automatic reconnect keeps the previous connection's strict xray routes on purpose, and the last-good fallback covers it); task 3.2 is satisfied by the existing `capture_dns=true|false` session-record field instead of a second `dns_capture` field.

### `h1` — tasks 2.2 — seam `S4-probe-notice`

- order: parallel, shard `process`; coder `rust-coder`; packages `v2ray-rs-process`
- sites:
  - `crates/process/src/manager.rs` · `ProcessManager::has_tun_runtime (new)` · anchor `pub fn with_tun(mut self, tun: Option<TunRuntime>) -> Self {` — new pub fn returning self.tun.is_some() (field `    tun: Option<TunRuntime>,`); no existing public accessor for tun
- work, in order:
  - tests first: has_tun_runtime_reflects_attached_runtime
  - manager.rs: add `pub fn has_tun_runtime(&self) -> bool { self.tun.is_some() }` next to state()
- names: `pub fn has_tun_runtime(&self) -> bool`
- verify: `make test-process && make clippy`

### `h2` — tasks 1.1, 1.2 — seam `S2-resolve`

- order: after `h1`, shared `crates/ui/src`, shard `ui`; coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/connection.rs` · `pin_hosts (new)` · anchor `const PIN_LOOKUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);` — new pure fn pin_hosts(candidates, subscriptions, manual_nodes, enabled_rules, settings) -> Vec<(String,u16)> next to PIN_LOOKUP_TIMEOUT: per candidate run resolve_effective_config + resolve_via_nodes (same as loop), skip when effective tun.enabled is false or backend is V2ray (mirror build_tun_runtime gate), skip IpAddr literals and hosts already in effective settings.dns.hosts (user overrides), dedupe
  - `crates/ui/src/connection.rs` · `tests (pin_hosts unit tests, new)` · anchor `fn ip_addressed_nodes_need_no_pin() {` — new #[test]s: TUN off -> empty; duplicate hosts deduped; via node included; user override skipped. Use node()/candidate()/tun_settings() helpers
  - `crates/ui/src/connection.rs` · `resolve_pins (new), replaces pin_node_addresses lookup body` · anchor `async fn pin_node_addresses(settings: &mut AppSettings, nodes: &[ProxyNode]) {` — new async resolve_pins(hosts, last_good: &[HostOverride], deadline) with injectable lookup fn; concurrent lookup_host, 16 in flight, one 5s overall deadline (reuse PIN_LOOKUP_TIMEOUT); failed/timed-out host takes last_good addresses, else absent + log::warn!("cannot pin {host}: ..."). Returns host->addrs map. pin_node_addresses removed (1.3)
  - `crates/ui/src/connection.rs` · `tests (resolve_pins, new)` · anchor `fn hostname_without_a_pin_leaves_capture_off() {` — new #[tokio::test] with injected lookup fn: failing host + last_good -> fallback used; timing-out host without last_good -> absent and logged (crate::logging::install_test_capture + lines_containing, as failover_is_traceable)
- work, in order:
  - tests first: pin_hosts_empty_without_tun, pin_hosts_empty_for_v2ray, pin_hosts_dedupes_shared_hostname, pin_hosts_includes_via_node, pin_hosts_skips_user_override, pin_hosts_skips_ip_literals, resolve_pins_uses_last_good_when_lookup_fails, resolve_pins_drops_timed_out_host_without_last_good (deadline 100 ms, asserts log `cannot pin slow.invalid: lookup timed out`), resolve_pins_caps_lookups_at_16 (40 hosts; injected lookup increments an AtomicUsize in-flight counter, records the max, sleeps 50 ms, decrements; assert max == 16), AtomicUsize max-in-flight == 16)
  - implement with tokio::task::JoinSet + Arc<tokio::sync::Semaphore>(16) + tokio::time::timeout_at over the join loop; hosts not joined by deadline are timed out (JoinSet dropped aborts them); no new dependency (futures is not a ui dep, crates/ui/Cargo.toml)
- names: `fn pin_hosts(candidates: &[ConnectionCandidate], subscriptions: &[Subscription], manual_nodes: &[ManualNode], enabled_rules: &[RoutingRule], settings: &AppSettings) -> Vec<(String, u16)>`; `fn pins_enabled(settings: &AppSettings) -> bool  // tun.enabled && backend_type != V2ray`; `async fn resolve_pins<F, Fut>(hosts: Vec<(String, u16)>, last_good: &[HostOverride], deadline: Duration, lookup: F) -> HashMap<String, Vec<IpAddr>> where F: Fn(String, u16) -> Fut + Clone + Send + 'static, Fut: Future<Output = std::io::Result<Vec<IpAddr>>> + Send + 'static`; `async fn os_lookup(host: String, port: u16) -> std::io::Result<Vec<IpAddr>>  // tokio::net::lookup_host`; `const PIN_LOOKUP_CONCURRENCY: usize = 16`; `log lines (keep exact, live check 4.2 greps them): `cannot pin {host}: lookup timed out`, `cannot pin {host}: {err}`; fallback: `pin {host}: using last good addresses``
- verify: `make test-ui && make clippy`

### `h3` — tasks 1.3, 2.1 — seam `S3-loop`

- order: after `h2`, shared `crates/ui/src`, shard `ui`; coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/connection.rs` · `spawn_with (pre-loop resolution)` · anchor `let total = candidates.len();` — before 'candidates loop (lifecycle lock already held): hosts = pin_hosts(&candidates, &subscriptions, &manual_nodes, &enabled_rules, &settings); resolved = resolve_pins(hosts, &last_good_pins, ...).await
  - `crates/ui/src/connection.rs` · `spawn_with candidate loop` · anchor `pin_node_addresses(&mut effective_settings, &nodes).await;` — replace per-candidate lookup with applying resolved map entries for this candidate's nodes into effective_settings.dns.hosts (HostOverride{domain, ip}); only when effective TUN applies; delete pin_node_addresses fn
  - `crates/ui/src/connection.rs` · `ConnectionRequest / destructure / test request()` · anchor `pub health_timing: HealthTiming,` — new field last_good_pins: Vec<HostOverride>; add to destructure in spawn_with (anchor `        health_timing,\n    } = request;`) and to tests fn request() (anchor `            health_timing: HealthTiming::default(),` inside tests; note same text also in app.rs)
  - `crates/ui/src/connection.rs` · `tests (two-candidate TUN-off stub, new)` · anchor `async fn two_candidates_first_exits_before_ready() {` — new stub test: two hostname candidates (Shadowsocks via node()), TUN off (singbox_settings/ready_singbox_settings), assert generated sing-box.json has no hosts entry for the hostname
  - `crates/ui/src/connection.rs` · `filter_reachable (new)` · anchor `fn hosts_cover_nodes(settings: &AppSettings, nodes: &[ProxyNode]) -> bool {` — new async filter_reachable(addrs: &[IpAddr], port: u16, per_connect: Duration, total: Duration) -> Vec<IpAddr>: concurrent tokio::net::TcpStream::connect under per-connect timeout, overall total timeout; keep connected; none -> return all
  - `crates/ui/src/connection.rs` · `tests (filter_reachable, new)` · anchor `fn wrong_family_pin_leaves_capture_off() {` — new tokio tests: TcpListener on 127.0.0.1:0, port reused for 127.0.0.2 (closed) -> only 127.0.0.1 kept; both closed -> both kept
- work, in order:
  - tests first: tun_off_connection_pins_no_hostname (two backend.log session records, each contains `nodes_pinned=false`; sing-box.json has no `localhost` host rule — assert on parsed dns section, not raw string, since inbounds carry 127.0.0.1), filter_reachable_keeps_only_listening_address, filter_reachable_keeps_all_when_none_accept; precondition assert at test start: std::net::ToSocketAddrs::to_socket_addrs(&("localhost", 0)) yields ≥1 address, so the test cannot pass vacuously
  - tests first (also): failover_resolves_every_candidate_host_before_first_start — two candidates candidate("failover-a.invalid"), candidate("failover-b.invalid"), xray TUN stub failing config check; install_test_capture(); one lines_containing("failover-") call (the start line reads `candidate start 1/2 label=failover-a.invalid` because node remark is None); assert both `cannot pin failover-a.invalid` and `cannot pin failover-b.invalid` lines precede the first `candidate start 1/2` line (old code logs the start line before its per-candidate lookup, so this fails on HEAD)
  - connection.rs: before 'candidates compute hosts + resolved; in loop replace line 221 with apply step (skip IP literals, user overrides present before this step, dedupe by host within nodes, port = first node with that host)
  - ConnectionRequest gains `pub last_good_pins: Vec<HostOverride>` here (destructured in spawn_with, passed to resolve_pins); tests fn request() sets `last_good_pins: Vec::new()`; app.rs start_connection sets `last_good_pins: Vec::new()` until h5 wires the store
- names: `async fn filter_reachable(addrs: &[IpAddr], port: u16, per_connect: Duration, total: Duration) -> Vec<IpAddr>`; `const PIN_PROBE_CONNECT_TIMEOUT: Duration = Duration::from_millis(1500)`; `const PIN_PROBE_TOTAL: Duration = Duration::from_secs(2)`; `session record token asserted: `nodes_pinned=false` (existing format connection.rs:267-268)`
- verify: `make test-ui && make clippy`

### `h4` — tasks 3.1, 3.2 — seam `S4-probe-notice`

- order: after `h3`, shared `crates/ui/src`, shard `ui`; coder `rust-coder`; packages `v2ray-rs-process`, `v2ray-rs-ui`
- sites:
  - `crates/ui/src/connection.rs` · `spawn_with candidate loop (probe before write_config)` · anchor `let pinned = hosts_cover_nodes(&effective_settings, &nodes);` — before this line / write_config: per candidate hostname, filter pinned addrs with filter_reachable(1.5s per connect, 2s total) on node.port(), rewrite effective_settings.dns.hosts; skip when parked.as_ref().is_some_and(|m| m.<tun accessor>()); log notice when none answered; pure skip predicate fn + unit test
  - `crates/ui/src/connection.rs` · `spawn_with once-per-attempt DNS-capture-off notice` · anchor `// Decided once per connection so the notice cannot repeat per candidate.` — follow STRICT_ROUTE_NOTICE pattern: bool flag outside loop; inside loop after `let pinned = ...` when backend Xray && effective tun.enabled && dns_hijack == DnsHijackMode::Hijack && !pinned && !already_noticed: log.append_line("notice", ..) on backend_log, sender.emit(AppMsg::ProcessLogLine(generation, "notice: DNS capture off for this session: <host> could not be resolved before connecting")), sender.emit(AppMsg::ShowToast(..)). Host = first node hostname not covered
  - `crates/ui/src/connection.rs` · `tests (stub, new)` · anchor `async fn xray_tun_session_record_carries_dns_decisions() {` — new stub test modeled on this one (xray stub, getcap stub printing cap_net_admin=ep, executable netctl, HostProbe via connect_with): two unpinned hostname candidates -> drain-like loop counts exactly one notice ProcessLogLine and one AppMsg::ShowToast
  - `crates/ui/src/connection.rs` · `session record capture_dns field (existing, unchanged)` · anchor `"hijack={} capture_dns={} strict={} nodes_pinned={pinned} profile={}",` — no production change; the 3.1 stub test asserts capture_dns=false in every backend.log session record
- work, in order:
  - tests first: probe_skipped_when_parked_manager_holds_tun; unpinned_candidates_notify_capture_off_once — xray stub exits 0 on `-test` and 1 otherwise (backend exits after launch), tun_settings() with backend Xray, interface_name `v2rsnotice0` (nonexistent), connect_with hook adding HostProbe{getcap stub printing `$1 cap_net_admin=ep`, helper: executable(..)} (capability gate precedes check_config) so each candidate passes config check, writes its session record, and fails with TunDeviceTimeout (not host-level → failover); candidates candidate("notice-a.invalid"), candidate("notice-b.invalid"); counts ProcessLogLine starting `notice: DNS capture off for this session: notice-a.invalid` == 1 and ShowToast == 1; backend.log holds the notice exactly once and exactly 2 session records, each containing `capture_dns=false` and `nodes_pinned=false`. Runtime ~25 s (10 s device wait per candidate + ≤5 s lookups); RECV_TIMEOUT must cover it — use a 60 s receive bound in this test
  - task 2.2 ui half (completes 2.2 after h1 added the accessor): fn probe_allowed(parked: Option<&ProcessManager>) -> bool = parked.is_none_or(|m| !m.has_tun_runtime())
  - loop: before write_config, when probe_allowed(parked.as_ref()) run filter_reachable for each pinned host concurrently (tokio::join via JoinSet), else keep all; on none-accepted log::warn!("pin probe: no address of {host} accepted on port {port}; keeping all")
  - loop: after build_tun_runtime compute notice condition; set capture_notice_sent
  - loop wiring test: probe_runs_before_write_config_when_allowed — TcpListener on 127.0.0.1:0 → port p; candidate built from a ShadowsocksConfig node with address "probe-a.invalid" and port p (node() fixture hard-codes 8388); xray TUN stub + HostProbe as above; let mut req = request(..); req.last_good_pins = vec![HostOverride 127.0.0.1, HostOverride 127.0.0.2 for probe-a.invalid]; spawn; wait for the terminal state message; read generated xray.json and assert dns.hosts["probe-a.invalid"] == "127.0.0.1" (a single kept address is emitted as a string, not an array)
- names: `fn probe_allowed(parked: Option<&ProcessManager>) -> bool`; `fn unpinned_host<'a>(settings: &AppSettings, nodes: &'a [ProxyNode]) -> Option<&'a str>  // hosts_cover_nodes becomes unpinned_host(..).is_none()`; `fn dns_capture_off_notice(host: &str) -> String  // exactly `notice: DNS capture off for this session: {host} could not be resolved before connecting` (design.md:30)`; `fn dns_capture_off_toast(host: &str) -> String  // `DNS capture is off for this session: {host} could not be resolved` (not fixed by the change; decided here)`; `AppMsg::ShowToast(String) existing app.rs:143`; `AppMsg::ProcessLogLine(u64, String) existing app.rs:130`
- verify: `make test-ui && make clippy`

### `h5` — tasks 3.3, 4.1 — seam `S5-last-good`

- order: after `h4`, shared `crates/ui/src`, shard `integration`; coder `rust-coder`; packages `v2ray-rs-ui`
- sites:
  - `crates/ui/src/app.rs` · `AppMsg::NodePins (new)` · anchor `TunGrantRequired(u64),` — new variant NodePins(u64, Vec<HostOverride>) (HostOverride derives Debug/Clone)
  - `crates/ui/src/connection.rs` · `spawn_with Running branch` · anchor `report(ProcessState::Running, Some(meta.clone()));` — after reporting Running emit AppMsg::NodePins(generation, applied pins = effective_settings.dns.hosts entries for node hostnames, excluding user overrides)
  - `crates/ui/src/app.rs` · `App struct + init` · anchor `grant_generation: Option<u64>,` — new field last_good_pins: Vec<HostOverride>; init next to `            connection_generation: 0,`
  - `crates/ui/src/app.rs` · `update handler` · anchor `AppMsg::TunGrantRequired(generation) => {` — new arm AppMsg::NodePins(generation, pins): store only when is_current_generation(generation, self.connection_generation), via pure helper fn apply_node_pins(store: &mut Vec<HostOverride>, message_generation: u64, current_generation: u64, pins: Vec<HostOverride>) mirroring error_toast_action
  - `crates/ui/src/app.rs` · `ConnectionRequest construction` · anchor `health_timing: HealthTiming::default(),` — add last_good_pins: self.last_good_pins.clone()
  - `crates/ui/src/app.rs` · `tests (pure helper, new)` · anchor `fn error_toast_action_arms_only_for_matching_generation() {` — new #[test]: stale generation ignored, current stored
- work, in order:
  - tests first (app.rs): node_pins_from_stale_generation_are_ignored, node_pins_replace_only_their_hostnames
  - connection.rs: collect applied Vec<HostOverride> per candidate; emit AppMsg::NodePins(generation, applied.clone()) after report(Running) when non-empty; resolve_pins gets &last_good_pins
  - app.rs: AppMsg::NodePins(u64, Vec<HostOverride>) variant, App field last_good_pins, handler calling apply_node_pins, start_connection passes last_good_pins: self.last_good_pins.clone() (replacing the Vec::new() from h3); match arms elsewhere already have catch-alls
  - 4.1: timeout 10m cargo test --workspace -- --test-threads=4
- names: `AppMsg::NodePins(u64, Vec<HostOverride>)`; `App field last_good_pins: Vec<HostOverride>`; `ConnectionRequest field pub last_good_pins: Vec<HostOverride>`; `fn apply_node_pins(store: &mut Vec<HostOverride>, message_generation: u64, current_generation: u64, pins: Vec<HostOverride>)`
- verify: `make test-ui && make clippy && timeout 10m cargo test --workspace -- --test-threads=4`

### Contracts

#### `S2-resolve` — tasks 1.1, 1.2

NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Pure host collection and concurrent bounded resolution in crates/ui/src/connection.rs, not yet wired into the loop.

States: `hosts (Vec<(String,u16)>)`, `resolved (HashMap<String, Vec<IpAddr>>)`

| input | state | effect | evidence |
|---|---|---|---|
| candidate with effective settings tun.enabled=false | `hosts` | no-op | tasks.md 1.1; spec 'Connections without TUN SHALL NOT pin' |
| effective backend V2ray (tun on) | `hosts` | no-op | design.md:25; build_tun_runtime connection.rs:546-553 |
| hostname node (candidate or via node from resolve_via_nodes over resolve_effective_config rules) | `hosts` | set | design.md:26; loop today connection.rs:205-218 |
| IP-literal node address | `hosts` | no-op | connection.rs:488-490 |
| hostname with an existing effective settings.dns.hosts entry (user override) | `hosts` | no-op | connection.rs:491-493 |
| same hostname on several nodes/candidates | `hosts` | no-op | dedupe by hostname, first (host,port) in candidate order wins |
| lookup Ok(non-empty) | `resolved[host]` | set | addresses deduped, lookup order kept |
| lookup Err or Ok(empty), last_good has host | `resolved[host]` | forced | tasks.md 1.2; design.md:29 — value = last_good ips parsed from HostOverride.ip |
| lookup unfinished at deadline, last_good has host | `resolved[host]` | forced | spec 'Last good addresses cover a failed lookup' |
| lookup Err/empty/timeout, no last_good | `resolved[host]` | clear | tasks.md 1.2 'absent and logged' |
| empty hosts | `resolved` | no-op | returns empty map immediately, no deadline wait |

Forbidden:
- more than 16 lookups in flight
- resolve_pins returning later than deadline + scheduling slack
- a last_good entry for a host whose lookup succeeded replacing the fresh answer
- an unparsable last_good ip string entering resolved

Seeding:
- pin_hosts: plain values — candidate(addr) fixtures (connection.rs:954-964), ManualNode{id, node, enabled:true} + RoutingRule with via_node=Some(ConnectionNodeRef::Manual{node_id}) enabled, AppSettings from tun_settings() (connection.rs:2238) with backend_type set; user override via settings.dns.hosts.push(HostOverride{..})
- resolve_pins: injected lookup closure keyed by host returning Ok(vec)/Err(io::Error)/tokio::time::sleep past deadline; log line via crate::logging::install_test_capture().lines_containing (logging.rs:133,174)

Budgets:
- PIN_LOOKUP_CONCURRENCY = 16 in flight
- PIN_LOOKUP_TIMEOUT = 5 s overall deadline (reuse existing const connection.rs:481, now meaning the whole batch)

#### `S3-loop` — tasks 1.3, 2.1

NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Wire resolve once before 'candidates (connection.rs:187) and apply per candidate; delete pin_node_addresses (connection.rs:483-517); add pure filter_reachable (not yet called).

States: `resolved_pins`, `candidate_hosts_pinned`, `candidate_hosts_unpinned`, `kept_addrs`

| input | state | effect | evidence |
|---|---|---|---|
| attempt start (after lifecycle lock, orphan reap, strict/v2ray notices) | `resolved_pins` | set | design.md:26; one resolve_pins(pin_hosts(..), &last_good_pins, PIN_LOOKUP_TIMEOUT, os_lookup) call before the loop |
| candidate, pins_enabled(effective)=false | `candidate_hosts_unpinned` | no-op | spec 'No pinning without TUN' |
| candidate, pins_enabled=true, hostname node in resolved, no user override | `candidate_hosts_pinned` | set | one HostOverride{domain, ip} per kept address, pushed before hosts_cover_nodes (connection.rs:225) and write_config (connection.rs:227) |
| hostname node absent from resolved | `candidate_hosts_unpinned` | set | node stays unpinned → nodes_pinned=false, capture off (connection.rs:524-534, 570-572) |
| filter_reachable: some connects succeed within budgets | `kept_addrs` | set | tasks.md 2.1; spec 'Unreachable pinned addresses are dropped' |
| filter_reachable: none succeed / all time out | `kept_addrs` | forced | spec 'No address answers' → all input addrs, input order |
| filter_reachable: empty input | `kept_addrs` | no-op | returns empty, no connect |

Forbidden:
- any lookup_host call inside 'candidates
- pin_node_addresses still present
- pins pushed for a candidate whose effective TUN is off
- filter_reachable reordering addresses or returning an empty vec for non-empty input

Seeding:
- 1.3 stub test: stub `[ "$1" = version ] && echo "sing-box version 1.13.0" && exit 0; [ "$1" = check ] && exit 0; exit 1`, singbox_settings() (TUN off), candidates vec![candidate("localhost"), candidate("localhost")] — localhost resolves from /etc/hosts without network, so the old per-candidate pin would have set nodes_pinned=true; drain(&rx); read backend.log
- 2.1: std/tokio TcpListener bound on 127.0.0.1:0 → port p; 127.0.0.2:p is closed (loopback /8 answers RST); both-closed case drops the listener first

Budgets:
- PIN_PROBE_CONNECT_TIMEOUT = 1500 ms per connect
- PIN_PROBE_TOTAL = 2 s per call (callers run a candidate's hosts concurrently so the candidate total stays 2 s)

#### `S4-probe-notice` — tasks 2.2, 3.1, 3.2

NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Task 2.2 spans chunks h1 (process accessor has_tun_runtime) and h4 (probe_allowed predicate + loop wiring); per-candidate probe of resolved addrs before write_config, skipped when parked holds a TUN runtime; once-per-attempt disabled-capture notice + toast; task 3.2 reported by the existing capture_dns session field, asserted by the 3.1 test.

States: `probe_allowed`, `probe_skipped`, `capture_notice_sent`, `capture_notice_pending`

| input | state | effect | evidence |
|---|---|---|---|
| parked = None | `probe_allowed` | set | tasks.md 2.2 |
| parked = Some(mgr), !mgr.has_tun_runtime() | `probe_allowed` | set | no routing state from this attempt |
| parked = Some(mgr), mgr.has_tun_runtime() | `probe_skipped` | set | design.md:28 probe would measure the kill switch; keep all addrs |
| candidate: backend Xray, effective tun on, dns_hijack Hijack, unpinned_host(..) = Some(host), !capture_notice_sent | `capture_notice_sent` | set | design.md:30; emits backend_log.append_line("notice", text), AppMsg::ProcessLogLine(generation, text), AppMsg::ShowToast(toast) |
| same condition, capture_notice_sent=true | `capture_notice_sent` | no-op | 'once per attempt even across candidates' |
| all hostname nodes pinned, or backend SingBox, or hijack Native/Disabled, or tun off | `capture_notice_pending` | no-op | capture is xray+Hijack only connection.rs:570-572 |

Forbidden:
- probe run while parked.has_tun_runtime()
- more than one notice or toast per spawned connection task
- notice emitted after the candidate's start begins (must precede start_with_connection, connection.rs:310)
- notice condition diverging from build_tun_runtime's capture_dns inputs

Seeding:
- predicate: ProcessManager::new(..) with and without .with_tun(Some(TunRuntime{backend: Xray, ..})) — TunRuntime fields are pub (tun.rs:31-45)
- 3.1 stub: xray stub `[ "$1" = version ] && echo "Xray 26.6.27" && exit 0; [ "$2" = -test ] && exit 0; exit 1`; tun_settings() + backend Xray + interface_name "v2rsnotice0" (nonexistent) + strict_route=false; connect_with(.., |mgr| mgr.with_host_probe(HostProbe{getcap: script printing "$1 cap_net_admin=ep", helper: executable(..)})) as in xray_tun_session_record_carries_dns_decisions, otherwise the capability gate before check_config fails host-level on candidate 1; candidates candidate("notice-a.invalid"), candidate("notice-b.invalid") → each writes its session record then fails TunDeviceTimeout (not host-level) → loop visits 2 candidates

Budgets:
- notice ≤ 1 and toast ≤ 1 per attempt
- probe 1.5 s/connect, 2 s total per candidate
- probe skipped → 0 connects

#### `S5-last-good` — tasks 3.3, 4.1

NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Loop sends applied pins on Running; App keeps them in memory per hostname and passes them into every ConnectionRequest; then workspace floor.

States: `App.last_good_pins`, `ConnectionRequest.last_good_pins`, `AppMsg::NodePins`

| input | state | effect | evidence |
|---|---|---|---|
| candidate reports Running (connection.rs:323) with non-empty applied pins | `AppMsg::NodePins` | set | tasks.md 3.3; sent right after report(Running) |
| Running with empty applied pins (TUN off / IP nodes) | `AppMsg::NodePins` | no-op | an empty message would carry nothing to keep |
| in-place respawn back to Running (connection.rs:441-442) | `AppMsg::NodePins` | no-op | same frozen pins; design non-goal re-resolve |
| NodePins(g, pins), g == connection_generation | `App.last_good_pins` | set | per hostname replace: drop existing entries whose domain is in pins, then append pins (spec: 'pinned for that hostname by the most recent session') |
| NodePins(g, pins), g != connection_generation | `App.last_good_pins` | no-op | is_current_generation app.rs:1831-1833 |
| start_connection | `ConnectionRequest.last_good_pins` | set | clone of App.last_good_pins at app.rs:648-663 |
| app restart | `App.last_good_pins` | clear | design.md:29 not persisted; init Vec::new() beside connection_generation: 0 (app.rs:1042) |

Forbidden:
- last_good_pins written to disk
- user host overrides (present in settings before pinning) included in NodePins
- stale-generation NodePins mutating the store

Seeding:
- pure helper only: fn apply_node_pins(store: &mut Vec<HostOverride>, message_generation: u64, current_generation: u64, pins: Vec<HostOverride>) called from the NodePins handler; tests build Vec<HostOverride> literals
- connection.rs test request() helper (connection.rs:974-995) gains last_good_pins: Vec::new()

Budgets:
- ≤ 1 NodePins per candidate Running report
- store unbounded in count but bounded by distinct node hostnames

### Floor

make fmt && make clippy && make test TEST_TIMEOUT=10m. Plus MANUAL task 4.2 live pass: xray TUN with a hostname node (capture_dns=true, pinned addresses accept on node port), TUN off (no node host entry), forced failover between two hostname nodes (no cannot pin lookup timed out).

### Requirements map

- For TUN connections only, the system SHALL resolve every hostname-addressed proxy node through the operating-system resolver and carry the answers, of both families, into the generated config as static host overrides, so the backend never has to resolve its own server through the tunnel it is building; each generator keeps the addresses its backend can use. → `pinned_hostname_arms_capture`, `pin_hosts_includes_via_node`
- Connections without TUN SHALL NOT pin node hostnames. → `pin_hosts_empty_without_tun`, `tun_off_connection_pins_no_hostname`
- The hostnames of all candidates of a connection attempt, including nodes their routing rules send traffic through, SHALL be resolved once, before the first candidate is started, and never while routing state installed by an earlier candidate of the same attempt is present. → `failover_resolves_every_candidate_host_before_first_start`, `pin_hosts_dedupes_shared_hostname`, `resolve_pins_caps_lookups_at_16`
- Before a candidate starts, the system SHALL test each of its pinned addresses with a TCP connection to the node's port, bounded by a short timeout, and SHALL keep only the addresses that accepted; when none accepted, or when a previously failed candidate's routing state is still installed, it SHALL keep all of them. → `filter_reachable_keeps_only_listening_address`, `filter_reachable_keeps_all_when_none_accept`, `probe_skipped_when_parked_manager_holds_tun`, `has_tun_runtime_reflects_attached_runtime`
- When a hostname cannot be resolved, the system SHALL use the addresses pinned for that hostname by the most recent session of the running application that reached `Running`, if any. → `resolve_pins_uses_last_good_when_lookup_fails`, `node_pins_from_stale_generation_are_ignored`, `node_pins_replace_only_their_hostnames`
- Kernel-side DNS capture SHALL be armed only when the generated config actually carries an override the backend can answer with for every hostname-addressed node, because capturing port 53 while the backend still needs a name resolved sends that lookup into the tunnel. → `hostname_without_a_pin_leaves_capture_off`, `wrong_family_pin_leaves_capture_off`, `pinned_hostname_arms_capture`
- When capture would otherwise be armed but a node is unpinned, the system SHALL notify the user once per connection attempt, write a notice naming the hostname to the process log stream, and record in the session record that DNS capture is off. → `unpinned_candidates_notify_capture_off_once`, `xray_tun_session_record_carries_dns_decisions`
- The TUN runtime SHALL be built from the same effective settings the config was generated from. → `xray_tun_session_record_carries_dns_decisions`
- **THEN** the system SHALL resolve it through the operating-system resolver before the route helper runs, and the generated config SHALL contain a host override for that hostname → `pin_hosts_includes_via_node`, `pinned_hostname_arms_capture`
- **THEN** the system SHALL NOT resolve it in advance and the generated config SHALL contain no host override for it that the user did not configure → `pin_hosts_empty_without_tun`, `tun_off_connection_pins_no_hostname`
- **THEN** that hostname SHALL already have been resolved before the first candidate started, and no lookup SHALL be made while the failed candidate's routing state is installed → `failover_resolves_every_candidate_host_before_first_start`, `pin_hosts_dedupes_shared_hostname`
- **THEN** the generated config SHALL pin only that address → `filter_reachable_keeps_only_listening_address`, `probe_runs_before_write_config_when_allowed`
- **THEN** the generated config SHALL pin all resolved addresses → `filter_reachable_keeps_all_when_none_accept`, `probe_skipped_when_parked_manager_holds_tun`
- **THEN** the reconnect SHALL pin the addresses the earlier session used → `resolve_pins_uses_last_good_when_lookup_fails`, `node_pins_replace_only_their_hostnames`, `node_pins_from_stale_generation_are_ignored`
- **THEN** the node SHALL count as unpinned and DNS capture SHALL stay off → `wrong_family_pin_leaves_capture_off`
- **THEN** the route helper SHALL be invoked without DNS capture, and the session SHALL still start → `hostname_without_a_pin_leaves_capture_off`
- **THEN** the user SHALL see a notification that DNS capture is off for the session, the process log SHALL contain a notice naming the hostname, and the session record SHALL state that DNS capture is off → `unpinned_candidates_notify_capture_off_once`
- **THEN** the route helper SHALL be configured from the same effective settings used to generate the config → `xray_tun_session_record_carries_dns_decisions`

### Test harness

- Stub — crates/ui/src/connection.rs:    struct Stub { — TempDir + AppPaths(AppProfile::Test) + backend script path
- stub(script) — crates/ui/src/connection.rs:    fn stub(script: &str) -> Stub { — writes executable #!/bin/sh backend, ensures dirs
- executable(dir, name) — crates/ui/src/connection.rs:    fn executable(dir: &std::path::Path, name: &str) -> PathBuf { — exit-0 script (fake netctl)
- candidate(address) — crates/ui/src/connection.rs:    fn candidate(address: &str) -> ConnectionCandidate { — Manual node_ref, Shadowsocks node
- xhttp_candidate(address) — crates/ui/src/connection.rs:    fn xhttp_candidate(address: &str) -> ConnectionCandidate { — VLESS xhttp candidate
- node(address) — crates/ui/src/connection.rs:    fn node(address: &str) -> ProxyNode { — Shadowsocks port 8388
- request(stub, settings, candidates) — crates/ui/src/connection.rs:    ) -> ConnectionRequest { — full ConnectionRequest with GENERATION=7, empty rules/subs/manual, host_has_ipv6 true
- connect / connect_with / connect_with_health — crates/ui/src/connection.rs:    fn connect_with( — spawn_with + relm4::channel receiver, optional manager hook / HealthTiming
- singbox_settings / ready_singbox_settings — crates/ui/src/connection.rs:    fn ready_singbox_settings() -> (std::net::TcpListener, AppSettings) { — sing-box settings, socks_port bound to live listener
- tun_settings / v2ray_settings / strict_route_settings — crates/ui/src/connection.rs:    fn tun_settings() -> AppSettings { — TUN enabled defaults; v2ray variant with ports/listen/tun flag
- strict_route_stub / sleeping_stub / dns_burst_stub — crates/ui/src/connection.rs:    fn sleeping_stub() -> Stub { — sing-box stubs that pass check and sleep / emit DNS burst
- capless_probe — crates/ui/src/connection.rs:    fn capless_probe(mgr: ProcessManager) -> ProcessManager { — HostProbe with /bin/true getcap+helper (no caps)
- next_state / drain / assert_nothing_after_terminal / wait_running / stop_and_wait — crates/ui/src/connection.rs:    async fn drain(rx: &relm4::Receiver<AppMsg>) -> (Option<ProcessState>, Vec<String>) { — receive loops; drain returns terminal + ProcessLogLine lines, ignores other msgs (extend for ShowToast counting)
- assert_error_terminal — crates/ui/src/connection.rs:    fn assert_error_terminal(terminal: Option<ProcessState>) { — asserts Error terminal
- attempts(marker) — crates/ui/src/connection.rs:    fn attempts(marker: &std::path::Path) -> usize { — counts stub launch marker lines
- FakeProxy / fake_proxy / health_settings — crates/ui/src/connection.rs:    async fn fake_proxy(healthy: bool) -> FakeProxy { — local HTTP responder counting accepts
- install_test_capture — crates/ui/src/logging.rs:pub(crate) fn install_test_capture() -> &'static TestLogCapture { — captures log macros; lines_containing(needle)
- write_script / manager_for — crates/process/src/manager.rs:    fn manager_for(dir: &tempfile::TempDir, script_body: &str) -> ProcessManager { — ProcessManager over stub script + {} config
- VERSION_STUB / SINGBOX_VERSION_STUB — crates/process/src/manager.rs:    const VERSION_STUB: &str = — version-subcommand shell prefix
- backend_log(dir) — crates/process/src/manager.rs:    fn backend_log(dir: &std::path::Path) -> Arc<RotatingFileWriter> { — RotatingFileWriter at dir/backend.log
- stub_helper(dir, body) — crates/process/src/manager.rs:    fn stub_helper(dir: &std::path::Path, body: &str) -> (PathBuf, PathBuf) { — fake netctl recording calls
- xray_on_lo(helper) — crates/process/src/manager.rs:    fn xray_on_lo(helper: PathBuf) -> TunRuntime { — xray TunRuntime on lo, capture_dns false, strict false
- read_lines / wait_for_lines — crates/process/src/manager.rs:    async fn wait_for_lines(path: &std::path::Path, n: usize) -> Vec<String> { — backend.log readers
- contains_line / drain_states — crates/process/src/manager.rs:    fn contains_line(mgr: &ProcessManager, content: &str) -> bool { — log buffer / state event helpers
- xray_rt(strict) — crates/process/src/tun.rs:    fn xray_rt(strict: bool) -> TunRuntime { — xray TunRuntime with capture_dns true
- Test-environment assumption: `.invalid` lookups (failover-, notice-, probe- hosts) rely on the OS resolver returning NXDOMAIN; a network forging UDP/53 answers flips nodes_pinned/notice/probe assertions

### Plan review

Reviewer `zarchitect`, verdict `pass` after 2 rounds. Round 1 blockers: the capture-off test seeded a config-check failure that writes no session record (reseeded to fail at the device wait after the record); the failover-ordering and notice tests shared `.invalid` hostnames in the process-global log capture (hostnames made unique, one `lines_containing("failover-")` filter). Round 2 warnings folded in: HostProbe hook for the capability gate, probe test port and string-valued `dns.hosts` entry, task 2.2 split across h1/h4.

Risks:

- Probe skip sees only this attempt's parked manager: on auto-reconnect under a previous connection's strict fallback, probes fail fast (unreachable) → keep-all path; worst case 2 s per candidate if drops instead of rejects. Mitigation: bounded by PIN_PROBE_TOTAL; acceptable, documented.
- has_tun_runtime is conservative: a sing-box or preflight-failed parked candidate with no live routes still disables the probe for later candidates. Mitigation: keep-all is the pre-change behavior.
- 3.1 stub test resolves `.invalid` via the OS resolver; slow resolvers cost up to 5 s (PIN_LOOKUP_TIMEOUT) inside RECV_TIMEOUT 20 s. Mitigation: RFC 6761 names fail locally on systemd-resolved/glibc.
- 1.3 test relies on /etc/hosts resolving localhost; absent in exotic sandboxes → test passes vacuously. Mitigation: assert tun-on positive control is covered by 3.1/pinned_hostname_arms_capture; optionally add a pin_hosts-level assertion.
- design.md anchors are stale at HEAD (connection.rs:153-156 → 219-225, :310-346 → 481-517, manager.rs:395-415 → 676-697). Mitigation: use anchors in this packet.
- Toast text is not fixed by the change; chosen here as `DNS capture is off for this session: {host} could not be resolved`. Mitigation: confirm wording at review.
- Hostnames shared by nodes on different ports are probed on the first node's port only. Mitigation: all transports dial address:port (design.md:8); mismatch is rare, keep-all fallback limits harm.
- 4.2 live verification needs real xray TUN, capabilities and a failover; not covered by the floor.
- Automatic reconnect: the previous connection strict xray routes stay installed during the up-front lookup (existing requirement Traffic stays blocked while an xray TUN session reconnects); lookups may fail there and the last-good fallback covers it. Spec delta scoped to earlier candidates of the same attempt.

## Plan appendix

## Plan appendix

```json
{
  "v": 2,
  "change": "harden-node-pinning",
  "baseSha": "8bc03f0e414aad1707b74afc7aa0c8d0704ddd17",
  "generatedAt": "2026-09-16T14:04:03.403Z",
  "tier": "heavy",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality",
    "perf"
  ],
  "chunks": [
    {
      "id": "h1",
      "taskIds": [
        "2.2"
      ],
      "seam": "S4-probe-notice",
      "contract": {
        "states": [
          "manager_tun_some",
          "manager_tun_none"
        ],
        "transitions": [
          {
            "input": "ProcessManager::new(..).with_tun(Some(rt))",
            "state": "manager_tun_some",
            "effect": "set",
            "evidence": "manager.rs with_tun"
          },
          {
            "input": "ProcessManager::new(..) / with_tun(None)",
            "state": "manager_tun_none",
            "effect": "set",
            "evidence": "manager.rs new"
          }
        ],
        "forbidden": [
          "has_tun_runtime reporting whether routes are actually up (it reports only the attached runtime; conservative by design)",
          "any change to write_session_record"
        ],
        "seeding": [
          "has_tun_runtime: ProcessManager::new(..) with/without .with_tun(Some(xray_on_lo(path)))"
        ],
        "budgets": [
          "none: accessor only"
        ],
        "names": [
          "pub fn has_tun_runtime(&self) -> bool"
        ],
        "refusals": [
          "none: no failure path added"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first: has_tun_runtime_reflects_attached_runtime",
        "manager.rs: add `pub fn has_tun_runtime(&self) -> bool { self.tun.is_some() }` next to state()"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "shard": "process",
      "pkgDirs": [
        "crates/process/src"
      ],
      "pkgs": [
        "v2ray-rs-process"
      ],
      "sites": [
        {
          "task": "2.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::has_tun_runtime (new)",
          "anchor": "    pub fn with_tun(mut self, tun: Option<TunRuntime>) -> Self {",
          "change": "new pub fn returning self.tun.is_some() (field `    tun: Option<TunRuntime>,`); no existing public accessor for tun"
        }
      ],
      "verify": "make test-process && make clippy"
    },
    {
      "id": "h2",
      "taskIds": [
        "1.1",
        "1.2"
      ],
      "seam": "S2-resolve",
      "contract": {
        "states": [
          "hosts (Vec<(String,u16)>)",
          "resolved (HashMap<String, Vec<IpAddr>>)"
        ],
        "transitions": [
          {
            "input": "candidate with effective settings tun.enabled=false",
            "state": "hosts",
            "effect": "no-op",
            "evidence": "tasks.md 1.1; spec 'Connections without TUN SHALL NOT pin'"
          },
          {
            "input": "effective backend V2ray (tun on)",
            "state": "hosts",
            "effect": "no-op",
            "evidence": "design.md:25; build_tun_runtime connection.rs:546-553"
          },
          {
            "input": "hostname node (candidate or via node from resolve_via_nodes over resolve_effective_config rules)",
            "state": "hosts",
            "effect": "set",
            "evidence": "design.md:26; loop today connection.rs:205-218"
          },
          {
            "input": "IP-literal node address",
            "state": "hosts",
            "effect": "no-op",
            "evidence": "connection.rs:488-490"
          },
          {
            "input": "hostname with an existing effective settings.dns.hosts entry (user override)",
            "state": "hosts",
            "effect": "no-op",
            "evidence": "connection.rs:491-493"
          },
          {
            "input": "same hostname on several nodes/candidates",
            "state": "hosts",
            "effect": "no-op",
            "evidence": "dedupe by hostname, first (host,port) in candidate order wins"
          },
          {
            "input": "lookup Ok(non-empty)",
            "state": "resolved[host]",
            "effect": "set",
            "evidence": "addresses deduped, lookup order kept"
          },
          {
            "input": "lookup Err or Ok(empty), last_good has host",
            "state": "resolved[host]",
            "effect": "forced",
            "evidence": "tasks.md 1.2; design.md:29 — value = last_good ips parsed from HostOverride.ip"
          },
          {
            "input": "lookup unfinished at deadline, last_good has host",
            "state": "resolved[host]",
            "effect": "forced",
            "evidence": "spec 'Last good addresses cover a failed lookup'"
          },
          {
            "input": "lookup Err/empty/timeout, no last_good",
            "state": "resolved[host]",
            "effect": "clear",
            "evidence": "tasks.md 1.2 'absent and logged'"
          },
          {
            "input": "empty hosts",
            "state": "resolved",
            "effect": "no-op",
            "evidence": "returns empty map immediately, no deadline wait"
          }
        ],
        "forbidden": [
          "more than 16 lookups in flight",
          "resolve_pins returning later than deadline + scheduling slack",
          "a last_good entry for a host whose lookup succeeded replacing the fresh answer",
          "an unparsable last_good ip string entering resolved"
        ],
        "seeding": [
          "pin_hosts: plain values — candidate(addr) fixtures (connection.rs:954-964), ManualNode{id, node, enabled:true} + RoutingRule with via_node=Some(ConnectionNodeRef::Manual{node_id}) enabled, AppSettings from tun_settings() (connection.rs:2238) with backend_type set; user override via settings.dns.hosts.push(HostOverride{..})",
          "resolve_pins: injected lookup closure keyed by host returning Ok(vec)/Err(io::Error)/tokio::time::sleep past deadline; log line via crate::logging::install_test_capture().lines_containing (logging.rs:133,174)"
        ],
        "budgets": [
          "PIN_LOOKUP_CONCURRENCY = 16 in flight",
          "PIN_LOOKUP_TIMEOUT = 5 s overall deadline (reuse existing const connection.rs:481, now meaning the whole batch)"
        ],
        "names": [
          "fn pin_hosts(candidates: &[ConnectionCandidate], subscriptions: &[Subscription], manual_nodes: &[ManualNode], enabled_rules: &[RoutingRule], settings: &AppSettings) -> Vec<(String, u16)>",
          "fn pins_enabled(settings: &AppSettings) -> bool  // tun.enabled && backend_type != V2ray",
          "async fn resolve_pins<F, Fut>(hosts: Vec<(String, u16)>, last_good: &[HostOverride], deadline: Duration, lookup: F) -> HashMap<String, Vec<IpAddr>> where F: Fn(String, u16) -> Fut + Clone + Send + 'static, Fut: Future<Output = std::io::Result<Vec<IpAddr>>> + Send + 'static",
          "async fn os_lookup(host: String, port: u16) -> std::io::Result<Vec<IpAddr>>  // tokio::net::lookup_host",
          "const PIN_LOOKUP_CONCURRENCY: usize = 16",
          "log lines (keep exact, live check 4.2 greps them): `cannot pin {host}: lookup timed out`, `cannot pin {host}: {err}`; fallback: `pin {host}: using last good addresses`"
        ],
        "refusals": [
          "none: failures degrade to absent entry + log::warn!, never an Err"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first: pin_hosts_empty_without_tun, pin_hosts_empty_for_v2ray, pin_hosts_dedupes_shared_hostname, pin_hosts_includes_via_node, pin_hosts_skips_user_override, pin_hosts_skips_ip_literals, resolve_pins_uses_last_good_when_lookup_fails, resolve_pins_drops_timed_out_host_without_last_good (deadline 100 ms, asserts log `cannot pin slow.invalid: lookup timed out`), resolve_pins_caps_lookups_at_16 (40 hosts; injected lookup increments an AtomicUsize in-flight counter, records the max, sleeps 50 ms, decrements; assert max == 16), AtomicUsize max-in-flight == 16)",
        "implement with tokio::task::JoinSet + Arc<tokio::sync::Semaphore>(16) + tokio::time::timeout_at over the join loop; hosts not joined by deadline are timed out (JoinSet dropped aborts them); no new dependency (futures is not a ui dep, crates/ui/Cargo.toml)"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "prev": "h1",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "pin_hosts (new)",
          "anchor": "const PIN_LOOKUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);",
          "change": "new pure fn pin_hosts(candidates, subscriptions, manual_nodes, enabled_rules, settings) -> Vec<(String,u16)> next to PIN_LOOKUP_TIMEOUT: per candidate run resolve_effective_config + resolve_via_nodes (same as loop), skip when effective tun.enabled is false or backend is V2ray (mirror build_tun_runtime gate), skip IpAddr literals and hosts already in effective settings.dns.hosts (user overrides), dedupe"
        },
        {
          "task": "1.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests (pin_hosts unit tests, new)",
          "anchor": "    fn ip_addressed_nodes_need_no_pin() {",
          "change": "new #[test]s: TUN off -> empty; duplicate hosts deduped; via node included; user override skipped. Use node()/candidate()/tun_settings() helpers"
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "resolve_pins (new), replaces pin_node_addresses lookup body",
          "anchor": "async fn pin_node_addresses(settings: &mut AppSettings, nodes: &[ProxyNode]) {",
          "change": "new async resolve_pins(hosts, last_good: &[HostOverride], deadline) with injectable lookup fn; concurrent lookup_host, 16 in flight, one 5s overall deadline (reuse PIN_LOOKUP_TIMEOUT); failed/timed-out host takes last_good addresses, else absent + log::warn!(\"cannot pin {host}: ...\"). Returns host->addrs map. pin_node_addresses removed (1.3)"
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests (resolve_pins, new)",
          "anchor": "    fn hostname_without_a_pin_leaves_capture_off() {",
          "change": "new #[tokio::test] with injected lookup fn: failing host + last_good -> fallback used; timing-out host without last_good -> absent and logged (crate::logging::install_test_capture + lines_containing, as failover_is_traceable)"
        }
      ],
      "verify": "make test-ui && make clippy"
    },
    {
      "id": "h3",
      "taskIds": [
        "1.3",
        "2.1"
      ],
      "seam": "S3-loop",
      "contract": {
        "states": [
          "resolved_pins",
          "candidate_hosts_pinned",
          "candidate_hosts_unpinned",
          "kept_addrs"
        ],
        "transitions": [
          {
            "input": "attempt start (after lifecycle lock, orphan reap, strict/v2ray notices)",
            "state": "resolved_pins",
            "effect": "set",
            "evidence": "design.md:26; one resolve_pins(pin_hosts(..), &last_good_pins, PIN_LOOKUP_TIMEOUT, os_lookup) call before the loop"
          },
          {
            "input": "candidate, pins_enabled(effective)=false",
            "state": "candidate_hosts_unpinned",
            "effect": "no-op",
            "evidence": "spec 'No pinning without TUN'"
          },
          {
            "input": "candidate, pins_enabled=true, hostname node in resolved, no user override",
            "state": "candidate_hosts_pinned",
            "effect": "set",
            "evidence": "one HostOverride{domain, ip} per kept address, pushed before hosts_cover_nodes (connection.rs:225) and write_config (connection.rs:227)"
          },
          {
            "input": "hostname node absent from resolved",
            "state": "candidate_hosts_unpinned",
            "effect": "set",
            "evidence": "node stays unpinned → nodes_pinned=false, capture off (connection.rs:524-534, 570-572)"
          },
          {
            "input": "filter_reachable: some connects succeed within budgets",
            "state": "kept_addrs",
            "effect": "set",
            "evidence": "tasks.md 2.1; spec 'Unreachable pinned addresses are dropped'"
          },
          {
            "input": "filter_reachable: none succeed / all time out",
            "state": "kept_addrs",
            "effect": "forced",
            "evidence": "spec 'No address answers' → all input addrs, input order"
          },
          {
            "input": "filter_reachable: empty input",
            "state": "kept_addrs",
            "effect": "no-op",
            "evidence": "returns empty, no connect"
          }
        ],
        "forbidden": [
          "any lookup_host call inside 'candidates",
          "pin_node_addresses still present",
          "pins pushed for a candidate whose effective TUN is off",
          "filter_reachable reordering addresses or returning an empty vec for non-empty input"
        ],
        "seeding": [
          "1.3 stub test: stub `[ \"$1\" = version ] && echo \"sing-box version 1.13.0\" && exit 0; [ \"$1\" = check ] && exit 0; exit 1`, singbox_settings() (TUN off), candidates vec![candidate(\"localhost\"), candidate(\"localhost\")] — localhost resolves from /etc/hosts without network, so the old per-candidate pin would have set nodes_pinned=true; drain(&rx); read backend.log",
          "2.1: std/tokio TcpListener bound on 127.0.0.1:0 → port p; 127.0.0.2:p is closed (loopback /8 answers RST); both-closed case drops the listener first"
        ],
        "budgets": [
          "PIN_PROBE_CONNECT_TIMEOUT = 1500 ms per connect",
          "PIN_PROBE_TOTAL = 2 s per call (callers run a candidate's hosts concurrently so the candidate total stays 2 s)"
        ],
        "names": [
          "async fn filter_reachable(addrs: &[IpAddr], port: u16, per_connect: Duration, total: Duration) -> Vec<IpAddr>",
          "const PIN_PROBE_CONNECT_TIMEOUT: Duration = Duration::from_millis(1500)",
          "const PIN_PROBE_TOTAL: Duration = Duration::from_secs(2)",
          "session record token asserted: `nodes_pinned=false` (existing format connection.rs:267-268)"
        ],
        "refusals": [
          "none"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first: tun_off_connection_pins_no_hostname (two backend.log session records, each contains `nodes_pinned=false`; sing-box.json has no `localhost` host rule — assert on parsed dns section, not raw string, since inbounds carry 127.0.0.1), filter_reachable_keeps_only_listening_address, filter_reachable_keeps_all_when_none_accept; precondition assert at test start: std::net::ToSocketAddrs::to_socket_addrs(&(\"localhost\", 0)) yields ≥1 address, so the test cannot pass vacuously",
        "tests first (also): failover_resolves_every_candidate_host_before_first_start — two candidates candidate(\"failover-a.invalid\"), candidate(\"failover-b.invalid\"), xray TUN stub failing config check; install_test_capture(); one lines_containing(\"failover-\") call (the start line reads `candidate start 1/2 label=failover-a.invalid` because node remark is None); assert both `cannot pin failover-a.invalid` and `cannot pin failover-b.invalid` lines precede the first `candidate start 1/2` line (old code logs the start line before its per-candidate lookup, so this fails on HEAD)",
        "connection.rs: before 'candidates compute hosts + resolved; in loop replace line 221 with apply step (skip IP literals, user overrides present before this step, dedupe by host within nodes, port = first node with that host)",
        "ConnectionRequest gains `pub last_good_pins: Vec<HostOverride>` here (destructured in spawn_with, passed to resolve_pins); tests fn request() sets `last_good_pins: Vec::new()`; app.rs start_connection sets `last_good_pins: Vec::new()` until h5 wires the store"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "prev": "h2",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "shard": "ui",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "1.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with (pre-loop resolution)",
          "anchor": "        let total = candidates.len();",
          "change": "before 'candidates loop (lifecycle lock already held): hosts = pin_hosts(&candidates, &subscriptions, &manual_nodes, &enabled_rules, &settings); resolved = resolve_pins(hosts, &last_good_pins, ...).await"
        },
        {
          "task": "1.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with candidate loop",
          "anchor": "            pin_node_addresses(&mut effective_settings, &nodes).await;",
          "change": "replace per-candidate lookup with applying resolved map entries for this candidate's nodes into effective_settings.dns.hosts (HostOverride{domain, ip}); only when effective TUN applies; delete pin_node_addresses fn"
        },
        {
          "task": "1.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "ConnectionRequest / destructure / test request()",
          "anchor": "    pub health_timing: HealthTiming,",
          "change": "new field last_good_pins: Vec<HostOverride>; add to destructure in spawn_with (anchor `        health_timing,\\n    } = request;`) and to tests fn request() (anchor `            health_timing: HealthTiming::default(),` inside tests; note same text also in app.rs)"
        },
        {
          "task": "1.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests (two-candidate TUN-off stub, new)",
          "anchor": "    async fn two_candidates_first_exits_before_ready() {",
          "change": "new stub test: two hostname candidates (Shadowsocks via node()), TUN off (singbox_settings/ready_singbox_settings), assert generated sing-box.json has no hosts entry for the hostname"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "filter_reachable (new)",
          "anchor": "fn hosts_cover_nodes(settings: &AppSettings, nodes: &[ProxyNode]) -> bool {",
          "change": "new async filter_reachable(addrs: &[IpAddr], port: u16, per_connect: Duration, total: Duration) -> Vec<IpAddr>: concurrent tokio::net::TcpStream::connect under per-connect timeout, overall total timeout; keep connected; none -> return all"
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests (filter_reachable, new)",
          "anchor": "    fn wrong_family_pin_leaves_capture_off() {",
          "change": "new tokio tests: TcpListener on 127.0.0.1:0, port reused for 127.0.0.2 (closed) -> only 127.0.0.1 kept; both closed -> both kept"
        }
      ],
      "verify": "make test-ui && make clippy"
    },
    {
      "id": "h4",
      "taskIds": [
        "3.1",
        "3.2"
      ],
      "seam": "S4-probe-notice",
      "contract": {
        "states": [
          "probe_allowed",
          "probe_skipped",
          "capture_notice_sent",
          "capture_notice_pending"
        ],
        "transitions": [
          {
            "input": "parked = None",
            "state": "probe_allowed",
            "effect": "set",
            "evidence": "tasks.md 2.2"
          },
          {
            "input": "parked = Some(mgr), !mgr.has_tun_runtime()",
            "state": "probe_allowed",
            "effect": "set",
            "evidence": "no routing state from this attempt"
          },
          {
            "input": "parked = Some(mgr), mgr.has_tun_runtime()",
            "state": "probe_skipped",
            "effect": "set",
            "evidence": "design.md:28 probe would measure the kill switch; keep all addrs"
          },
          {
            "input": "candidate: backend Xray, effective tun on, dns_hijack Hijack, unpinned_host(..) = Some(host), !capture_notice_sent",
            "state": "capture_notice_sent",
            "effect": "set",
            "evidence": "design.md:30; emits backend_log.append_line(\"notice\", text), AppMsg::ProcessLogLine(generation, text), AppMsg::ShowToast(toast)"
          },
          {
            "input": "same condition, capture_notice_sent=true",
            "state": "capture_notice_sent",
            "effect": "no-op",
            "evidence": "'once per attempt even across candidates'"
          },
          {
            "input": "all hostname nodes pinned, or backend SingBox, or hijack Native/Disabled, or tun off",
            "state": "capture_notice_pending",
            "effect": "no-op",
            "evidence": "capture is xray+Hijack only connection.rs:570-572"
          }
        ],
        "forbidden": [
          "probe run while parked.has_tun_runtime()",
          "more than one notice or toast per spawned connection task",
          "notice emitted after the candidate's start begins (must precede start_with_connection, connection.rs:310)",
          "notice condition diverging from build_tun_runtime's capture_dns inputs"
        ],
        "seeding": [
          "predicate: ProcessManager::new(..) with and without .with_tun(Some(TunRuntime{backend: Xray, ..})) — TunRuntime fields are pub (tun.rs:31-45)",
          "3.1 stub: xray stub `[ \"$1\" = version ] && echo \"Xray 26.6.27\" && exit 0; [ \"$2\" = -test ] && exit 0; exit 1`; tun_settings() + backend Xray + interface_name \"v2rsnotice0\" (nonexistent) + strict_route=false; connect_with(.., |mgr| mgr.with_host_probe(HostProbe{getcap: script printing \"$1 cap_net_admin=ep\", helper: executable(..)})) as in xray_tun_session_record_carries_dns_decisions, otherwise the capability gate before check_config fails host-level on candidate 1; candidates candidate(\"notice-a.invalid\"), candidate(\"notice-b.invalid\") → each writes its session record then fails TunDeviceTimeout (not host-level) → loop visits 2 candidates"
        ],
        "budgets": [
          "notice ≤ 1 and toast ≤ 1 per attempt",
          "probe 1.5 s/connect, 2 s total per candidate",
          "probe skipped → 0 connects"
        ],
        "names": [
          "fn probe_allowed(parked: Option<&ProcessManager>) -> bool",
          "fn unpinned_host<'a>(settings: &AppSettings, nodes: &'a [ProxyNode]) -> Option<&'a str>  // hosts_cover_nodes becomes unpinned_host(..).is_none()",
          "fn dns_capture_off_notice(host: &str) -> String  // exactly `notice: DNS capture off for this session: {host} could not be resolved before connecting` (design.md:30)",
          "fn dns_capture_off_toast(host: &str) -> String  // `DNS capture is off for this session: {host} could not be resolved` (not fixed by the change; decided here)",
          "AppMsg::ShowToast(String) existing app.rs:143",
          "AppMsg::ProcessLogLine(u64, String) existing app.rs:130"
        ],
        "refusals": [
          "none: probe and notice never fail the connect"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first: probe_skipped_when_parked_manager_holds_tun; unpinned_candidates_notify_capture_off_once — xray stub exits 0 on `-test` and 1 otherwise (backend exits after launch), tun_settings() with backend Xray, interface_name `v2rsnotice0` (nonexistent), connect_with hook adding HostProbe{getcap stub printing `$1 cap_net_admin=ep`, helper: executable(..)} (capability gate precedes check_config) so each candidate passes config check, writes its session record, and fails with TunDeviceTimeout (not host-level → failover); candidates candidate(\"notice-a.invalid\"), candidate(\"notice-b.invalid\"); counts ProcessLogLine starting `notice: DNS capture off for this session: notice-a.invalid` == 1 and ShowToast == 1; backend.log holds the notice exactly once and exactly 2 session records, each containing `capture_dns=false` and `nodes_pinned=false`. Runtime ~25 s (10 s device wait per candidate + ≤5 s lookups); RECV_TIMEOUT must cover it — use a 60 s receive bound in this test",
        "task 2.2 ui half (completes 2.2 after h1 added the accessor): fn probe_allowed(parked: Option<&ProcessManager>) -> bool = parked.is_none_or(|m| !m.has_tun_runtime())",
        "loop: before write_config, when probe_allowed(parked.as_ref()) run filter_reachable for each pinned host concurrently (tokio::join via JoinSet), else keep all; on none-accepted log::warn!(\"pin probe: no address of {host} accepted on port {port}; keeping all\")",
        "loop: after build_tun_runtime compute notice condition; set capture_notice_sent",
        "loop wiring test: probe_runs_before_write_config_when_allowed — TcpListener on 127.0.0.1:0 → port p; candidate built from a ShadowsocksConfig node with address \"probe-a.invalid\" and port p (node() fixture hard-codes 8388); xray TUN stub + HostProbe as above; let mut req = request(..); req.last_good_pins = vec![HostOverride 127.0.0.1, HostOverride 127.0.0.2 for probe-a.invalid]; spawn; wait for the terminal state message; read generated xray.json and assert dns.hosts[\"probe-a.invalid\"] == \"127.0.0.1\" (a single kept address is emitted as a string, not an array)"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "prev": "h3",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
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
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with candidate loop (probe before write_config)",
          "anchor": "            let pinned = hosts_cover_nodes(&effective_settings, &nodes);",
          "change": "before this line / write_config: per candidate hostname, filter pinned addrs with filter_reachable(1.5s per connect, 2s total) on node.port(), rewrite effective_settings.dns.hosts; skip when parked.as_ref().is_some_and(|m| m.<tun accessor>()); log notice when none answered; pure skip predicate fn + unit test"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with once-per-attempt DNS-capture-off notice",
          "anchor": "        // Decided once per connection so the notice cannot repeat per candidate.",
          "change": "follow STRICT_ROUTE_NOTICE pattern: bool flag outside loop; inside loop after `let pinned = ...` when backend Xray && effective tun.enabled && dns_hijack == DnsHijackMode::Hijack && !pinned && !already_noticed: log.append_line(\"notice\", ..) on backend_log, sender.emit(AppMsg::ProcessLogLine(generation, \"notice: DNS capture off for this session: <host> could not be resolved before connecting\")), sender.emit(AppMsg::ShowToast(..)). Host = first node hostname not covered"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests (stub, new)",
          "anchor": "    async fn xray_tun_session_record_carries_dns_decisions() {",
          "change": "new stub test modeled on this one (xray stub, getcap stub printing cap_net_admin=ep, executable netctl, HostProbe via connect_with): two unpinned hostname candidates -> drain-like loop counts exactly one notice ProcessLogLine and one AppMsg::ShowToast"
        },
        {
          "task": "3.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "session record capture_dns field (existing, unchanged)",
          "anchor": "\"hijack={} capture_dns={} strict={} nodes_pinned={pinned} profile={}\",",
          "change": "no production change; the 3.1 stub test asserts capture_dns=false in every backend.log session record"
        }
      ],
      "verify": "make test-ui && make clippy"
    },
    {
      "id": "h5",
      "taskIds": [
        "3.3",
        "4.1"
      ],
      "seam": "S5-last-good",
      "contract": {
        "states": [
          "App.last_good_pins",
          "ConnectionRequest.last_good_pins",
          "AppMsg::NodePins"
        ],
        "transitions": [
          {
            "input": "candidate reports Running (connection.rs:323) with non-empty applied pins",
            "state": "AppMsg::NodePins",
            "effect": "set",
            "evidence": "tasks.md 3.3; sent right after report(Running)"
          },
          {
            "input": "Running with empty applied pins (TUN off / IP nodes)",
            "state": "AppMsg::NodePins",
            "effect": "no-op",
            "evidence": "an empty message would carry nothing to keep"
          },
          {
            "input": "in-place respawn back to Running (connection.rs:441-442)",
            "state": "AppMsg::NodePins",
            "effect": "no-op",
            "evidence": "same frozen pins; design non-goal re-resolve"
          },
          {
            "input": "NodePins(g, pins), g == connection_generation",
            "state": "App.last_good_pins",
            "effect": "set",
            "evidence": "per hostname replace: drop existing entries whose domain is in pins, then append pins (spec: 'pinned for that hostname by the most recent session')"
          },
          {
            "input": "NodePins(g, pins), g != connection_generation",
            "state": "App.last_good_pins",
            "effect": "no-op",
            "evidence": "is_current_generation app.rs:1831-1833"
          },
          {
            "input": "start_connection",
            "state": "ConnectionRequest.last_good_pins",
            "effect": "set",
            "evidence": "clone of App.last_good_pins at app.rs:648-663"
          },
          {
            "input": "app restart",
            "state": "App.last_good_pins",
            "effect": "clear",
            "evidence": "design.md:29 not persisted; init Vec::new() beside connection_generation: 0 (app.rs:1042)"
          }
        ],
        "forbidden": [
          "last_good_pins written to disk",
          "user host overrides (present in settings before pinning) included in NodePins",
          "stale-generation NodePins mutating the store"
        ],
        "seeding": [
          "pure helper only: fn apply_node_pins(store: &mut Vec<HostOverride>, message_generation: u64, current_generation: u64, pins: Vec<HostOverride>) called from the NodePins handler; tests build Vec<HostOverride> literals",
          "connection.rs test request() helper (connection.rs:974-995) gains last_good_pins: Vec::new()"
        ],
        "budgets": [
          "≤ 1 NodePins per candidate Running report",
          "store unbounded in count but bounded by distinct node hostnames"
        ],
        "names": [
          "AppMsg::NodePins(u64, Vec<HostOverride>)",
          "App field last_good_pins: Vec<HostOverride>",
          "ConnectionRequest field pub last_good_pins: Vec<HostOverride>",
          "fn apply_node_pins(store: &mut Vec<HostOverride>, message_generation: u64, current_generation: u64, pins: Vec<HostOverride>)"
        ],
        "refusals": [
          "none"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first (app.rs): node_pins_from_stale_generation_are_ignored, node_pins_replace_only_their_hostnames",
        "connection.rs: collect applied Vec<HostOverride> per candidate; emit AppMsg::NodePins(generation, applied.clone()) after report(Running) when non-empty; resolve_pins gets &last_good_pins",
        "app.rs: AppMsg::NodePins(u64, Vec<HostOverride>) variant, App field last_good_pins, handler calling apply_node_pins, start_connection passes last_good_pins: self.last_good_pins.clone() (replacing the Vec::new() from h3); match arms elsewhere already have catch-alls",
        "4.1: timeout 10m cargo test --workspace -- --test-threads=4"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "prev": "h4",
      "sharedPkg": "crates/ui/src",
      "parallel": false,
      "shard": "",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::NodePins (new)",
          "anchor": "    TunGrantRequired(u64),",
          "change": "new variant NodePins(u64, Vec<HostOverride>) (HostOverride derives Debug/Clone)"
        },
        {
          "task": "3.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with Running branch",
          "anchor": "                    report(ProcessState::Running, Some(meta.clone()));",
          "change": "after reporting Running emit AppMsg::NodePins(generation, applied pins = effective_settings.dns.hosts entries for node hostnames, excluding user overrides)"
        },
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "App struct + init",
          "anchor": "    grant_generation: Option<u64>,",
          "change": "new field last_good_pins: Vec<HostOverride>; init next to `            connection_generation: 0,`"
        },
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "update handler",
          "anchor": "            AppMsg::TunGrantRequired(generation) => {",
          "change": "new arm AppMsg::NodePins(generation, pins): store only when is_current_generation(generation, self.connection_generation), via pure helper fn apply_node_pins(store: &mut Vec<HostOverride>, message_generation: u64, current_generation: u64, pins: Vec<HostOverride>) mirroring error_toast_action"
        },
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "ConnectionRequest construction",
          "anchor": "                health_timing: HealthTiming::default(),",
          "change": "add last_good_pins: self.last_good_pins.clone()"
        },
        {
          "task": "3.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "tests (pure helper, new)",
          "anchor": "    fn error_toast_action_arms_only_for_matching_generation() {",
          "change": "new #[test]: stale generation ignored, current stored"
        }
      ],
      "verify": "make test-ui && make clippy && timeout 10m cargo test --workspace -- --test-threads=4"
    }
  ],
  "seams": [
    {
      "id": "S2-resolve",
      "tasks": [
        "1.1",
        "1.2"
      ],
      "summary": "NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Pure host collection and concurrent bounded resolution in crates/ui/src/connection.rs, not yet wired into the loop.",
      "contract": {
        "states": [
          "hosts (Vec<(String,u16)>)",
          "resolved (HashMap<String, Vec<IpAddr>>)"
        ],
        "transitions": [
          {
            "input": "candidate with effective settings tun.enabled=false",
            "state": "hosts",
            "effect": "no-op",
            "evidence": "tasks.md 1.1; spec 'Connections without TUN SHALL NOT pin'"
          },
          {
            "input": "effective backend V2ray (tun on)",
            "state": "hosts",
            "effect": "no-op",
            "evidence": "design.md:25; build_tun_runtime connection.rs:546-553"
          },
          {
            "input": "hostname node (candidate or via node from resolve_via_nodes over resolve_effective_config rules)",
            "state": "hosts",
            "effect": "set",
            "evidence": "design.md:26; loop today connection.rs:205-218"
          },
          {
            "input": "IP-literal node address",
            "state": "hosts",
            "effect": "no-op",
            "evidence": "connection.rs:488-490"
          },
          {
            "input": "hostname with an existing effective settings.dns.hosts entry (user override)",
            "state": "hosts",
            "effect": "no-op",
            "evidence": "connection.rs:491-493"
          },
          {
            "input": "same hostname on several nodes/candidates",
            "state": "hosts",
            "effect": "no-op",
            "evidence": "dedupe by hostname, first (host,port) in candidate order wins"
          },
          {
            "input": "lookup Ok(non-empty)",
            "state": "resolved[host]",
            "effect": "set",
            "evidence": "addresses deduped, lookup order kept"
          },
          {
            "input": "lookup Err or Ok(empty), last_good has host",
            "state": "resolved[host]",
            "effect": "forced",
            "evidence": "tasks.md 1.2; design.md:29 — value = last_good ips parsed from HostOverride.ip"
          },
          {
            "input": "lookup unfinished at deadline, last_good has host",
            "state": "resolved[host]",
            "effect": "forced",
            "evidence": "spec 'Last good addresses cover a failed lookup'"
          },
          {
            "input": "lookup Err/empty/timeout, no last_good",
            "state": "resolved[host]",
            "effect": "clear",
            "evidence": "tasks.md 1.2 'absent and logged'"
          },
          {
            "input": "empty hosts",
            "state": "resolved",
            "effect": "no-op",
            "evidence": "returns empty map immediately, no deadline wait"
          }
        ],
        "forbidden": [
          "more than 16 lookups in flight",
          "resolve_pins returning later than deadline + scheduling slack",
          "a last_good entry for a host whose lookup succeeded replacing the fresh answer",
          "an unparsable last_good ip string entering resolved"
        ],
        "seeding": [
          "pin_hosts: plain values — candidate(addr) fixtures (connection.rs:954-964), ManualNode{id, node, enabled:true} + RoutingRule with via_node=Some(ConnectionNodeRef::Manual{node_id}) enabled, AppSettings from tun_settings() (connection.rs:2238) with backend_type set; user override via settings.dns.hosts.push(HostOverride{..})",
          "resolve_pins: injected lookup closure keyed by host returning Ok(vec)/Err(io::Error)/tokio::time::sleep past deadline; log line via crate::logging::install_test_capture().lines_containing (logging.rs:133,174)"
        ],
        "budgets": [
          "PIN_LOOKUP_CONCURRENCY = 16 in flight",
          "PIN_LOOKUP_TIMEOUT = 5 s overall deadline (reuse existing const connection.rs:481, now meaning the whole batch)"
        ],
        "names": [
          "fn pin_hosts(candidates: &[ConnectionCandidate], subscriptions: &[Subscription], manual_nodes: &[ManualNode], enabled_rules: &[RoutingRule], settings: &AppSettings) -> Vec<(String, u16)>",
          "fn pins_enabled(settings: &AppSettings) -> bool  // tun.enabled && backend_type != V2ray",
          "async fn resolve_pins<F, Fut>(hosts: Vec<(String, u16)>, last_good: &[HostOverride], deadline: Duration, lookup: F) -> HashMap<String, Vec<IpAddr>> where F: Fn(String, u16) -> Fut + Clone + Send + 'static, Fut: Future<Output = std::io::Result<Vec<IpAddr>>> + Send + 'static",
          "async fn os_lookup(host: String, port: u16) -> std::io::Result<Vec<IpAddr>>  // tokio::net::lookup_host",
          "const PIN_LOOKUP_CONCURRENCY: usize = 16",
          "log lines (keep exact, live check 4.2 greps them): `cannot pin {host}: lookup timed out`, `cannot pin {host}: {err}`; fallback: `pin {host}: using last good addresses`"
        ],
        "refusals": [
          "none: failures degrade to absent entry + log::warn!, never an Err"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first: pin_hosts_empty_without_tun, pin_hosts_empty_for_v2ray, pin_hosts_dedupes_shared_hostname, pin_hosts_includes_via_node, pin_hosts_skips_user_override, pin_hosts_skips_ip_literals, resolve_pins_uses_last_good_when_lookup_fails, resolve_pins_drops_timed_out_host_without_last_good (deadline 100 ms, asserts log `cannot pin slow.invalid: lookup timed out`), resolve_pins_caps_lookups_at_16 (40 hosts; injected lookup increments an AtomicUsize in-flight counter, records the max, sleeps 50 ms, decrements; assert max == 16), AtomicUsize max-in-flight == 16)",
        "implement with tokio::task::JoinSet + Arc<tokio::sync::Semaphore>(16) + tokio::time::timeout_at over the join loop; hosts not joined by deadline are timed out (JoinSet dropped aborts them); no new dependency (futures is not a ui dep, crates/ui/Cargo.toml)"
      ]
    },
    {
      "id": "S3-loop",
      "tasks": [
        "1.3",
        "2.1"
      ],
      "summary": "NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Wire resolve once before 'candidates (connection.rs:187) and apply per candidate; delete pin_node_addresses (connection.rs:483-517); add pure filter_reachable (not yet called).",
      "contract": {
        "states": [
          "resolved_pins",
          "candidate_hosts_pinned",
          "candidate_hosts_unpinned",
          "kept_addrs"
        ],
        "transitions": [
          {
            "input": "attempt start (after lifecycle lock, orphan reap, strict/v2ray notices)",
            "state": "resolved_pins",
            "effect": "set",
            "evidence": "design.md:26; one resolve_pins(pin_hosts(..), &last_good_pins, PIN_LOOKUP_TIMEOUT, os_lookup) call before the loop"
          },
          {
            "input": "candidate, pins_enabled(effective)=false",
            "state": "candidate_hosts_unpinned",
            "effect": "no-op",
            "evidence": "spec 'No pinning without TUN'"
          },
          {
            "input": "candidate, pins_enabled=true, hostname node in resolved, no user override",
            "state": "candidate_hosts_pinned",
            "effect": "set",
            "evidence": "one HostOverride{domain, ip} per kept address, pushed before hosts_cover_nodes (connection.rs:225) and write_config (connection.rs:227)"
          },
          {
            "input": "hostname node absent from resolved",
            "state": "candidate_hosts_unpinned",
            "effect": "set",
            "evidence": "node stays unpinned → nodes_pinned=false, capture off (connection.rs:524-534, 570-572)"
          },
          {
            "input": "filter_reachable: some connects succeed within budgets",
            "state": "kept_addrs",
            "effect": "set",
            "evidence": "tasks.md 2.1; spec 'Unreachable pinned addresses are dropped'"
          },
          {
            "input": "filter_reachable: none succeed / all time out",
            "state": "kept_addrs",
            "effect": "forced",
            "evidence": "spec 'No address answers' → all input addrs, input order"
          },
          {
            "input": "filter_reachable: empty input",
            "state": "kept_addrs",
            "effect": "no-op",
            "evidence": "returns empty, no connect"
          }
        ],
        "forbidden": [
          "any lookup_host call inside 'candidates",
          "pin_node_addresses still present",
          "pins pushed for a candidate whose effective TUN is off",
          "filter_reachable reordering addresses or returning an empty vec for non-empty input"
        ],
        "seeding": [
          "1.3 stub test: stub `[ \"$1\" = version ] && echo \"sing-box version 1.13.0\" && exit 0; [ \"$1\" = check ] && exit 0; exit 1`, singbox_settings() (TUN off), candidates vec![candidate(\"localhost\"), candidate(\"localhost\")] — localhost resolves from /etc/hosts without network, so the old per-candidate pin would have set nodes_pinned=true; drain(&rx); read backend.log",
          "2.1: std/tokio TcpListener bound on 127.0.0.1:0 → port p; 127.0.0.2:p is closed (loopback /8 answers RST); both-closed case drops the listener first"
        ],
        "budgets": [
          "PIN_PROBE_CONNECT_TIMEOUT = 1500 ms per connect",
          "PIN_PROBE_TOTAL = 2 s per call (callers run a candidate's hosts concurrently so the candidate total stays 2 s)"
        ],
        "names": [
          "async fn filter_reachable(addrs: &[IpAddr], port: u16, per_connect: Duration, total: Duration) -> Vec<IpAddr>",
          "const PIN_PROBE_CONNECT_TIMEOUT: Duration = Duration::from_millis(1500)",
          "const PIN_PROBE_TOTAL: Duration = Duration::from_secs(2)",
          "session record token asserted: `nodes_pinned=false` (existing format connection.rs:267-268)"
        ],
        "refusals": [
          "none"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first: tun_off_connection_pins_no_hostname (two backend.log session records, each contains `nodes_pinned=false`; sing-box.json has no `localhost` host rule — assert on parsed dns section, not raw string, since inbounds carry 127.0.0.1), filter_reachable_keeps_only_listening_address, filter_reachable_keeps_all_when_none_accept; precondition assert at test start: std::net::ToSocketAddrs::to_socket_addrs(&(\"localhost\", 0)) yields ≥1 address, so the test cannot pass vacuously",
        "tests first (also): failover_resolves_every_candidate_host_before_first_start — two candidates candidate(\"failover-a.invalid\"), candidate(\"failover-b.invalid\"), xray TUN stub failing config check; install_test_capture(); one lines_containing(\"failover-\") call (the start line reads `candidate start 1/2 label=failover-a.invalid` because node remark is None); assert both `cannot pin failover-a.invalid` and `cannot pin failover-b.invalid` lines precede the first `candidate start 1/2` line (old code logs the start line before its per-candidate lookup, so this fails on HEAD)",
        "connection.rs: before 'candidates compute hosts + resolved; in loop replace line 221 with apply step (skip IP literals, user overrides present before this step, dedupe by host within nodes, port = first node with that host)",
        "ConnectionRequest gains `pub last_good_pins: Vec<HostOverride>` here (destructured in spawn_with, passed to resolve_pins); tests fn request() sets `last_good_pins: Vec::new()`; app.rs start_connection sets `last_good_pins: Vec::new()` until h5 wires the store"
      ]
    },
    {
      "id": "S4-probe-notice",
      "tasks": [
        "2.2",
        "3.1",
        "3.2"
      ],
      "summary": "NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Task 2.2 spans chunks h1 (process accessor has_tun_runtime) and h4 (probe_allowed predicate + loop wiring); per-candidate probe of resolved addrs before write_config, skipped when parked holds a TUN runtime; once-per-attempt disabled-capture notice + toast; task 3.2 reported by the existing capture_dns session field, asserted by the 3.1 test.",
      "contract": {
        "states": [
          "probe_allowed",
          "probe_skipped",
          "capture_notice_sent",
          "capture_notice_pending"
        ],
        "transitions": [
          {
            "input": "parked = None",
            "state": "probe_allowed",
            "effect": "set",
            "evidence": "tasks.md 2.2"
          },
          {
            "input": "parked = Some(mgr), !mgr.has_tun_runtime()",
            "state": "probe_allowed",
            "effect": "set",
            "evidence": "no routing state from this attempt"
          },
          {
            "input": "parked = Some(mgr), mgr.has_tun_runtime()",
            "state": "probe_skipped",
            "effect": "set",
            "evidence": "design.md:28 probe would measure the kill switch; keep all addrs"
          },
          {
            "input": "candidate: backend Xray, effective tun on, dns_hijack Hijack, unpinned_host(..) = Some(host), !capture_notice_sent",
            "state": "capture_notice_sent",
            "effect": "set",
            "evidence": "design.md:30; emits backend_log.append_line(\"notice\", text), AppMsg::ProcessLogLine(generation, text), AppMsg::ShowToast(toast)"
          },
          {
            "input": "same condition, capture_notice_sent=true",
            "state": "capture_notice_sent",
            "effect": "no-op",
            "evidence": "'once per attempt even across candidates'"
          },
          {
            "input": "all hostname nodes pinned, or backend SingBox, or hijack Native/Disabled, or tun off",
            "state": "capture_notice_pending",
            "effect": "no-op",
            "evidence": "capture is xray+Hijack only connection.rs:570-572"
          }
        ],
        "forbidden": [
          "probe run while parked.has_tun_runtime()",
          "more than one notice or toast per spawned connection task",
          "notice emitted after the candidate's start begins (must precede start_with_connection, connection.rs:310)",
          "notice condition diverging from build_tun_runtime's capture_dns inputs"
        ],
        "seeding": [
          "predicate: ProcessManager::new(..) with and without .with_tun(Some(TunRuntime{backend: Xray, ..})) — TunRuntime fields are pub (tun.rs:31-45)",
          "3.1 stub: xray stub `[ \"$1\" = version ] && echo \"Xray 26.6.27\" && exit 0; [ \"$2\" = -test ] && exit 0; exit 1`; tun_settings() + backend Xray + interface_name \"v2rsnotice0\" (nonexistent) + strict_route=false; connect_with(.., |mgr| mgr.with_host_probe(HostProbe{getcap: script printing \"$1 cap_net_admin=ep\", helper: executable(..)})) as in xray_tun_session_record_carries_dns_decisions, otherwise the capability gate before check_config fails host-level on candidate 1; candidates candidate(\"notice-a.invalid\"), candidate(\"notice-b.invalid\") → each writes its session record then fails TunDeviceTimeout (not host-level) → loop visits 2 candidates"
        ],
        "budgets": [
          "notice ≤ 1 and toast ≤ 1 per attempt",
          "probe 1.5 s/connect, 2 s total per candidate",
          "probe skipped → 0 connects"
        ],
        "names": [
          "fn probe_allowed(parked: Option<&ProcessManager>) -> bool",
          "fn unpinned_host<'a>(settings: &AppSettings, nodes: &'a [ProxyNode]) -> Option<&'a str>  // hosts_cover_nodes becomes unpinned_host(..).is_none()",
          "fn dns_capture_off_notice(host: &str) -> String  // exactly `notice: DNS capture off for this session: {host} could not be resolved before connecting` (design.md:30)",
          "fn dns_capture_off_toast(host: &str) -> String  // `DNS capture is off for this session: {host} could not be resolved` (not fixed by the change; decided here)",
          "AppMsg::ShowToast(String) existing app.rs:143",
          "AppMsg::ProcessLogLine(u64, String) existing app.rs:130"
        ],
        "refusals": [
          "none: probe and notice never fail the connect"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first: probe_skipped_when_parked_manager_holds_tun; unpinned_candidates_notify_capture_off_once — xray stub exits 0 on `-test` and 1 otherwise (backend exits after launch), tun_settings() with backend Xray, interface_name `v2rsnotice0` (nonexistent), connect_with hook adding HostProbe{getcap stub printing `$1 cap_net_admin=ep`, helper: executable(..)} (capability gate precedes check_config) so each candidate passes config check, writes its session record, and fails with TunDeviceTimeout (not host-level → failover); candidates candidate(\"notice-a.invalid\"), candidate(\"notice-b.invalid\"); counts ProcessLogLine starting `notice: DNS capture off for this session: notice-a.invalid` == 1 and ShowToast == 1; backend.log holds the notice exactly once and exactly 2 session records, each containing `capture_dns=false` and `nodes_pinned=false`. Runtime ~25 s (10 s device wait per candidate + ≤5 s lookups); RECV_TIMEOUT must cover it — use a 60 s receive bound in this test",
        "task 2.2 ui half (completes 2.2 after h1 added the accessor): fn probe_allowed(parked: Option<&ProcessManager>) -> bool = parked.is_none_or(|m| !m.has_tun_runtime())",
        "loop: before write_config, when probe_allowed(parked.as_ref()) run filter_reachable for each pinned host concurrently (tokio::join via JoinSet), else keep all; on none-accepted log::warn!(\"pin probe: no address of {host} accepted on port {port}; keeping all\")",
        "loop: after build_tun_runtime compute notice condition; set capture_notice_sent",
        "loop wiring test: probe_runs_before_write_config_when_allowed — TcpListener on 127.0.0.1:0 → port p; candidate built from a ShadowsocksConfig node with address \"probe-a.invalid\" and port p (node() fixture hard-codes 8388); xray TUN stub + HostProbe as above; let mut req = request(..); req.last_good_pins = vec![HostOverride 127.0.0.1, HostOverride 127.0.0.2 for probe-a.invalid]; spawn; wait for the terminal state message; read generated xray.json and assert dns.hosts[\"probe-a.invalid\"] == \"127.0.0.1\" (a single kept address is emitted as a string, not an array)"
      ]
    },
    {
      "id": "S5-last-good",
      "tasks": [
        "3.3",
        "4.1"
      ],
      "summary": "NO-RED-WAIVER: rust, no test-writer agent; NO-TESTER-WAIVER: rust, verify runs in the floor. Loop sends applied pins on Running; App keeps them in memory per hostname and passes them into every ConnectionRequest; then workspace floor.",
      "contract": {
        "states": [
          "App.last_good_pins",
          "ConnectionRequest.last_good_pins",
          "AppMsg::NodePins"
        ],
        "transitions": [
          {
            "input": "candidate reports Running (connection.rs:323) with non-empty applied pins",
            "state": "AppMsg::NodePins",
            "effect": "set",
            "evidence": "tasks.md 3.3; sent right after report(Running)"
          },
          {
            "input": "Running with empty applied pins (TUN off / IP nodes)",
            "state": "AppMsg::NodePins",
            "effect": "no-op",
            "evidence": "an empty message would carry nothing to keep"
          },
          {
            "input": "in-place respawn back to Running (connection.rs:441-442)",
            "state": "AppMsg::NodePins",
            "effect": "no-op",
            "evidence": "same frozen pins; design non-goal re-resolve"
          },
          {
            "input": "NodePins(g, pins), g == connection_generation",
            "state": "App.last_good_pins",
            "effect": "set",
            "evidence": "per hostname replace: drop existing entries whose domain is in pins, then append pins (spec: 'pinned for that hostname by the most recent session')"
          },
          {
            "input": "NodePins(g, pins), g != connection_generation",
            "state": "App.last_good_pins",
            "effect": "no-op",
            "evidence": "is_current_generation app.rs:1831-1833"
          },
          {
            "input": "start_connection",
            "state": "ConnectionRequest.last_good_pins",
            "effect": "set",
            "evidence": "clone of App.last_good_pins at app.rs:648-663"
          },
          {
            "input": "app restart",
            "state": "App.last_good_pins",
            "effect": "clear",
            "evidence": "design.md:29 not persisted; init Vec::new() beside connection_generation: 0 (app.rs:1042)"
          }
        ],
        "forbidden": [
          "last_good_pins written to disk",
          "user host overrides (present in settings before pinning) included in NodePins",
          "stale-generation NodePins mutating the store"
        ],
        "seeding": [
          "pure helper only: fn apply_node_pins(store: &mut Vec<HostOverride>, message_generation: u64, current_generation: u64, pins: Vec<HostOverride>) called from the NodePins handler; tests build Vec<HostOverride> literals",
          "connection.rs test request() helper (connection.rs:974-995) gains last_good_pins: Vec::new()"
        ],
        "budgets": [
          "≤ 1 NodePins per candidate Running report",
          "store unbounded in count but bounded by distinct node hostnames"
        ],
        "names": [
          "AppMsg::NodePins(u64, Vec<HostOverride>)",
          "App field last_good_pins: Vec<HostOverride>",
          "ConnectionRequest field pub last_good_pins: Vec<HostOverride>",
          "fn apply_node_pins(store: &mut Vec<HostOverride>, message_generation: u64, current_generation: u64, pins: Vec<HostOverride>)"
        ],
        "refusals": [
          "none"
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "tests first (app.rs): node_pins_from_stale_generation_are_ignored, node_pins_replace_only_their_hostnames",
        "connection.rs: collect applied Vec<HostOverride> per candidate; emit AppMsg::NodePins(generation, applied.clone()) after report(Running) when non-empty; resolve_pins gets &last_good_pins",
        "app.rs: AppMsg::NodePins(u64, Vec<HostOverride>) variant, App field last_good_pins, handler calling apply_node_pins, start_connection passes last_good_pins: self.last_good_pins.clone() (replacing the Vec::new() from h3); match arms elsewhere already have catch-alls",
        "4.1: timeout 10m cargo test --workspace -- --test-threads=4"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "For TUN connections only, the system SHALL resolve every hostname-addressed proxy node through the operating-system resolver and carry the answers, of both families, into the generated config as static host overrides, so the backend never has to resolve its own server through the tunnel it is building; each generator keeps the addresses its backend can use.",
      "tests": [
        "pinned_hostname_arms_capture",
        "pin_hosts_includes_via_node"
      ]
    },
    {
      "shall": "Connections without TUN SHALL NOT pin node hostnames.",
      "tests": [
        "pin_hosts_empty_without_tun",
        "tun_off_connection_pins_no_hostname"
      ]
    },
    {
      "shall": "The hostnames of all candidates of a connection attempt, including nodes their routing rules send traffic through, SHALL be resolved once, before the first candidate is started, and never while routing state installed by an earlier candidate of the same attempt is present.",
      "tests": [
        "failover_resolves_every_candidate_host_before_first_start",
        "pin_hosts_dedupes_shared_hostname",
        "resolve_pins_caps_lookups_at_16"
      ]
    },
    {
      "shall": "Before a candidate starts, the system SHALL test each of its pinned addresses with a TCP connection to the node's port, bounded by a short timeout, and SHALL keep only the addresses that accepted; when none accepted, or when a previously failed candidate's routing state is still installed, it SHALL keep all of them.",
      "tests": [
        "filter_reachable_keeps_only_listening_address",
        "filter_reachable_keeps_all_when_none_accept",
        "probe_skipped_when_parked_manager_holds_tun",
        "has_tun_runtime_reflects_attached_runtime"
      ]
    },
    {
      "shall": "When a hostname cannot be resolved, the system SHALL use the addresses pinned for that hostname by the most recent session of the running application that reached `Running`, if any.",
      "tests": [
        "resolve_pins_uses_last_good_when_lookup_fails",
        "node_pins_from_stale_generation_are_ignored",
        "node_pins_replace_only_their_hostnames"
      ]
    },
    {
      "shall": "Kernel-side DNS capture SHALL be armed only when the generated config actually carries an override the backend can answer with for every hostname-addressed node, because capturing port 53 while the backend still needs a name resolved sends that lookup into the tunnel.",
      "tests": [
        "hostname_without_a_pin_leaves_capture_off",
        "wrong_family_pin_leaves_capture_off",
        "pinned_hostname_arms_capture"
      ]
    },
    {
      "shall": "When capture would otherwise be armed but a node is unpinned, the system SHALL notify the user once per connection attempt, write a notice naming the hostname to the process log stream, and record in the session record that DNS capture is off.",
      "tests": [
        "unpinned_candidates_notify_capture_off_once",
        "xray_tun_session_record_carries_dns_decisions"
      ]
    },
    {
      "shall": "The TUN runtime SHALL be built from the same effective settings the config was generated from.",
      "tests": [
        "xray_tun_session_record_carries_dns_decisions"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL resolve it through the operating-system resolver before the route helper runs, and the generated config SHALL contain a host override for that hostname",
      "tests": [
        "pin_hosts_includes_via_node",
        "pinned_hostname_arms_capture"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL NOT resolve it in advance and the generated config SHALL contain no host override for it that the user did not configure",
      "tests": [
        "pin_hosts_empty_without_tun",
        "tun_off_connection_pins_no_hostname"
      ]
    },
    {
      "shall": "- **THEN** that hostname SHALL already have been resolved before the first candidate started, and no lookup SHALL be made while the failed candidate's routing state is installed",
      "tests": [
        "failover_resolves_every_candidate_host_before_first_start",
        "pin_hosts_dedupes_shared_hostname"
      ]
    },
    {
      "shall": "- **THEN** the generated config SHALL pin only that address",
      "tests": [
        "filter_reachable_keeps_only_listening_address",
        "probe_runs_before_write_config_when_allowed"
      ]
    },
    {
      "shall": "- **THEN** the generated config SHALL pin all resolved addresses",
      "tests": [
        "filter_reachable_keeps_all_when_none_accept",
        "probe_skipped_when_parked_manager_holds_tun"
      ]
    },
    {
      "shall": "- **THEN** the reconnect SHALL pin the addresses the earlier session used",
      "tests": [
        "resolve_pins_uses_last_good_when_lookup_fails",
        "node_pins_replace_only_their_hostnames",
        "node_pins_from_stale_generation_are_ignored"
      ]
    },
    {
      "shall": "- **THEN** the node SHALL count as unpinned and DNS capture SHALL stay off",
      "tests": [
        "wrong_family_pin_leaves_capture_off"
      ]
    },
    {
      "shall": "- **THEN** the route helper SHALL be invoked without DNS capture, and the session SHALL still start",
      "tests": [
        "hostname_without_a_pin_leaves_capture_off"
      ]
    },
    {
      "shall": "- **THEN** the user SHALL see a notification that DNS capture is off for the session, the process log SHALL contain a notice naming the hostname, and the session record SHALL state that DNS capture is off",
      "tests": [
        "unpinned_candidates_notify_capture_off_once"
      ]
    },
    {
      "shall": "- **THEN** the route helper SHALL be configured from the same effective settings used to generate the config",
      "tests": [
        "xray_tun_session_record_carries_dns_decisions"
      ]
    }
  ],
  "testHarness": [
    "Stub — crates/ui/src/connection.rs:    struct Stub { — TempDir + AppPaths(AppProfile::Test) + backend script path",
    "stub(script) — crates/ui/src/connection.rs:    fn stub(script: &str) -> Stub { — writes executable #!/bin/sh backend, ensures dirs",
    "executable(dir, name) — crates/ui/src/connection.rs:    fn executable(dir: &std::path::Path, name: &str) -> PathBuf { — exit-0 script (fake netctl)",
    "candidate(address) — crates/ui/src/connection.rs:    fn candidate(address: &str) -> ConnectionCandidate { — Manual node_ref, Shadowsocks node",
    "xhttp_candidate(address) — crates/ui/src/connection.rs:    fn xhttp_candidate(address: &str) -> ConnectionCandidate { — VLESS xhttp candidate",
    "node(address) — crates/ui/src/connection.rs:    fn node(address: &str) -> ProxyNode { — Shadowsocks port 8388",
    "request(stub, settings, candidates) — crates/ui/src/connection.rs:    ) -> ConnectionRequest { — full ConnectionRequest with GENERATION=7, empty rules/subs/manual, host_has_ipv6 true",
    "connect / connect_with / connect_with_health — crates/ui/src/connection.rs:    fn connect_with( — spawn_with + relm4::channel receiver, optional manager hook / HealthTiming",
    "singbox_settings / ready_singbox_settings — crates/ui/src/connection.rs:    fn ready_singbox_settings() -> (std::net::TcpListener, AppSettings) { — sing-box settings, socks_port bound to live listener",
    "tun_settings / v2ray_settings / strict_route_settings — crates/ui/src/connection.rs:    fn tun_settings() -> AppSettings { — TUN enabled defaults; v2ray variant with ports/listen/tun flag",
    "strict_route_stub / sleeping_stub / dns_burst_stub — crates/ui/src/connection.rs:    fn sleeping_stub() -> Stub { — sing-box stubs that pass check and sleep / emit DNS burst",
    "capless_probe — crates/ui/src/connection.rs:    fn capless_probe(mgr: ProcessManager) -> ProcessManager { — HostProbe with /bin/true getcap+helper (no caps)",
    "next_state / drain / assert_nothing_after_terminal / wait_running / stop_and_wait — crates/ui/src/connection.rs:    async fn drain(rx: &relm4::Receiver<AppMsg>) -> (Option<ProcessState>, Vec<String>) { — receive loops; drain returns terminal + ProcessLogLine lines, ignores other msgs (extend for ShowToast counting)",
    "assert_error_terminal — crates/ui/src/connection.rs:    fn assert_error_terminal(terminal: Option<ProcessState>) { — asserts Error terminal",
    "attempts(marker) — crates/ui/src/connection.rs:    fn attempts(marker: &std::path::Path) -> usize { — counts stub launch marker lines",
    "FakeProxy / fake_proxy / health_settings — crates/ui/src/connection.rs:    async fn fake_proxy(healthy: bool) -> FakeProxy { — local HTTP responder counting accepts",
    "install_test_capture — crates/ui/src/logging.rs:pub(crate) fn install_test_capture() -> &'static TestLogCapture { — captures log macros; lines_containing(needle)",
    "write_script / manager_for — crates/process/src/manager.rs:    fn manager_for(dir: &tempfile::TempDir, script_body: &str) -> ProcessManager { — ProcessManager over stub script + {} config",
    "VERSION_STUB / SINGBOX_VERSION_STUB — crates/process/src/manager.rs:    const VERSION_STUB: &str = — version-subcommand shell prefix",
    "backend_log(dir) — crates/process/src/manager.rs:    fn backend_log(dir: &std::path::Path) -> Arc<RotatingFileWriter> { — RotatingFileWriter at dir/backend.log",
    "stub_helper(dir, body) — crates/process/src/manager.rs:    fn stub_helper(dir: &std::path::Path, body: &str) -> (PathBuf, PathBuf) { — fake netctl recording calls",
    "xray_on_lo(helper) — crates/process/src/manager.rs:    fn xray_on_lo(helper: PathBuf) -> TunRuntime { — xray TunRuntime on lo, capture_dns false, strict false",
    "read_lines / wait_for_lines — crates/process/src/manager.rs:    async fn wait_for_lines(path: &std::path::Path, n: usize) -> Vec<String> { — backend.log readers",
    "contains_line / drain_states — crates/process/src/manager.rs:    fn contains_line(mgr: &ProcessManager, content: &str) -> bool { — log buffer / state event helpers",
    "xray_rt(strict) — crates/process/src/tun.rs:    fn xray_rt(strict: bool) -> TunRuntime { — xray TunRuntime with capture_dns true",
    "Test-environment assumption: `.invalid` lookups (failover-, notice-, probe- hosts) rely on the OS resolver returning NXDOMAIN; a network forging UDP/53 answers flips nodes_pinned/notice/probe assertions"
  ],
  "floor": "make fmt && make clippy && make test TEST_TIMEOUT=10m. Plus MANUAL task 4.2 live pass: xray TUN with a hostname node (capture_dns=true, pinned addresses accept on node port), TUN off (no node host entry), forced failover between two hostname nodes (no cannot pin lookup timed out).",
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
