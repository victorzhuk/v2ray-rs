# Tasks: Real Delay Latency Test

## 1. Core models and persistence (crates/core)

- [x] 1.1 Add `last_real_delay_ms: Option<u64>` to `SubscriptionNode` in `crates/core/src/models/subscription.rs` with `#[serde(default, skip_serializing_if = "Option::is_none")]`; add the equivalent field to the manual-node record if it lives in a separate struct.
- [x] 1.2 Add `RealDelaySettings { enabled: bool, test_url: String, timeout_ms: u32, use_for_lowest_latency: bool }` in `crates/core/src/models/settings.rs`, with `Default` returning `enabled = true`, `test_url = "https://www.gstatic.com/generate_204"`, `timeout_ms = 5000`, `use_for_lowest_latency = false`.
- [x] 1.3 Add `pub real_delay: RealDelaySettings` to `AppSettings` with `#[serde(default)]`; ensure legacy `settings.toml` without the section loads via existing wire-shim pattern used for `dns`.
- [x] 1.4 Add URL validation for `test_url` (must be `http://` or `https://`, parsable by `url::Url`) on the settings setter path; reject other schemes with a descriptive error reused by the UI.
- [x] 1.5 Extend the latency-snapshot serialization (`persistence.rs`) so each entry carries `real_ms: Option<u64>` alongside `tcp_ms`; verify forward/backward compatibility with a round-trip test that loads a snapshot missing the new field.
- [x] 1.6 Hydrate `SubscriptionNode::last_real_delay_ms` from the snapshot on startup in the same place that hydrates `last_latency_ms`.
- [x] 1.7 Preserve `last_real_delay_ms` across subscription refresh — extend the existing merge-by-stable-id path so the new field is carried over.
- [x] 1.8 Update `ConnectionPlanner` in `crates/core/src/resolve.rs`: when `settings.real_delay.use_for_lowest_latency` is true, sort lowest-latency candidates by `last_real_delay_ms` (then by `last_latency_ms` for nodes lacking real delay, then unknown last).
- [x] 1.9 Unit tests for: settings (de)serialization with and without the new section, URL validation, snapshot migration (legacy + new format), refresh preserves Real Delay sample, planner ordering with mixed-sample populations.

## 2. Probe config generators (crates/core/src/config)

- [x] 2.1 Add `probe.rs` defining `trait ProbeConfigGenerator { fn generate(&self, nodes: &[&SubscriptionNode], api_port: u16, test_url: &str, timeout_ms: u32) -> serde_json::Value; fn outbound_tag(&self, idx: usize) -> String; }` and `pub fn probe_generator_for(backend: BackendType) -> Option<Box<dyn ProbeConfigGenerator>>` (returns `None` for unsupported backends).
- [x] 2.2 Implement `SingboxProbeGenerator`: emit a config with no inbounds, the candidate outbounds (reuse existing outbound-emission helpers from `singbox.rs`), no routing rules, and an `experimental.clash_api.external_controller = "127.0.0.1:<api_port>"` block. Tag outbounds as `probe-<idx>` for stable mapping.
- [x] 2.3 Implement `XrayProbeGenerator`: emit a config with no inbounds, the candidate outbounds, an `observatory` (or `burstObservatory` when emitting for xray ≥ 1.8) block listing all probe tags with `probeUrl = test_url` and `probeInterval = "500ms"`, plus a `stats` + `api` block bound to `127.0.0.1:<api_port>`.
- [x] 2.4 Unit tests per generator: snapshot-test the produced JSON for a representative VLESS + Trojan + Shadowsocks mix; assert API binding is on `127.0.0.1`, that there are no user-facing inbounds, and that every node appears as an outbound with the expected probe tag.

## 3. Isolated probe runner (crates/process)

- [x] 3.1 Add `crates/process/src/probe.rs` with `ProbeRunner { binary: PathBuf, config_path: PathBuf, api_port: u16, child: Option<Child> }` and a `ProbeError` enum (spawn, api-not-ready, exited-early, killed).
- [x] 3.2 Implement `ProbeRunner::start(&mut self)` reusing the ETXTBSY retry logic from `ProcessManager::spawn` (extract a shared `spawn_with_etxtbsy_retry` helper used by both).
- [x] 3.3 Implement `wait_ready(&mut self, timeout: Duration)`: poll `TcpStream::connect(("127.0.0.1", self.api_port))` with a short backoff until success or timeout; if the child exits during the wait, return `ExitedEarly` with captured stderr.
- [x] 3.4 Implement `stop(&mut self)`: SIGTERM → wait 5 s → SIGKILL; do not write any PID file, do not publish process events.
- [x] 3.5 Implement `Drop for ProbeRunner` that sends SIGKILL synchronously to any surviving child to prevent orphans on caller panic or app shutdown.
- [x] 3.6 On Linux, set `prctl(PR_SET_PDEATHSIG, SIGTERM)` in the child via `pre_exec` so the kernel cleans up the probe when its parent (v2ray-rs) dies.
- [x] 3.7 Unit / integration tests: spawn a no-op `/bin/sleep` as a fake backend to verify start → wait_ready timeout → stop path; verify `Drop` kills the child.

## 4. Real Delay orchestration (crates/subscription)

- [x] 4.1 Add `crates/subscription/src/real_delay.rs` exporting `pub async fn measure_real_delay(backend: &BackendInfo, nodes: &[&SubscriptionNode], cfg: &RealDelaySettings, paths: &AppPaths) -> Vec<Option<u64>>`.
- [x] 4.2 Inside `measure_real_delay`: pick a free localhost port (`TcpListener::bind("127.0.0.1:0")` then immediately drop); generate the probe config via `probe_generator_for(backend.kind)` into a `tempfile::NamedTempFile` in `runtime_dir`.
- [x] 4.3 Spawn `ProbeRunner`, call `wait_ready(Duration::from_secs(2))`; on failure, return `vec![None; nodes.len()]` and surface the diagnostic upward.
- [x] 4.4 sing-box code path: for each `nodes[i]`, issue `GET http://127.0.0.1:<port>/proxies/probe-<i>/delay?url=<encoded_url>&timeout=<ms>` in parallel with `tokio::spawn` + `reqwest::Client` (reuse the existing client from the subscription crate). Parse `{ "delay": <u64> }` on success; treat 408 / 504 / non-2xx as `None`.
- [x] 4.5 xray code path: after `wait_ready`, sleep `timeout_ms + 1500 ms`, then `GET /stats/observation` (or equivalent gRPC-over-HTTP endpoint exposed by the configured API) and map each tag's `delay` field to the node index.
- [x] 4.6 Always call `runner.stop().await` (also on early-return paths via a `scopeguard`-style helper or explicit `defer`-equivalent in an async block).
- [x] 4.7 Add a process-wide `tokio::sync::Mutex<()>` (`REAL_DELAY_LOCK`) guarding the whole session so at most one ephemeral backend exists at a time; document the rationale in a comment.
- [x] 4.8 Add `BackendInfo::supports_real_delay()` (or equivalent capability check) that returns `false` for sing-box without `with_clash_api` (detect by spawning the binary with `--help` once and caching), and for v2ray/xray builds without observatory.
- [x] 4.9 Unit tests with a mock HTTP server (use `wiremock` or a hand-rolled `axum` listener) standing in for the Clash API; cover success, timeout, partial failures, and probe-runner-crash scenarios. Integration test gated behind `#[ignore]` that spawns a real `sing-box` if available on the build machine.

## 5. UI integration (crates/ui)

- [x] 5.1 In `crates/ui/src/subscriptions.rs`, add a "Test Real Delay" action to the per-subscription menu and the per-selected-nodes context menu, gated on `BackendInfo::supports_real_delay()`; when unsupported, render the action insensitive with a tooltip explaining why.
- [x] 5.2 Display Real Delay results in the existing latency badge as `tcp NN · real MMM` when a Real Delay sample exists; show just `tcp NN` otherwise. Use the same color thresholds as TCP (green / yellow / red) keyed off the Real Delay number when present.
- [x] 5.3 Add a "Sort by Real Delay" toggle to the existing latency-sort menu; respect it across re-renders by extending `capture_expanded` / state preservation.
- [x] 5.4 In `crates/ui/src/settings.rs`, add a "Real Delay" group to the `AdwPreferencesPage` with:
  - a `AdwSwitchRow` for `enabled`,
  - a `AdwEntryRow` for `test_url` with inline validation (live-call into the core URL validator),
  - a `AdwSpinRow` (or numeric entry) for `timeout_ms` (range 500–60000),
  - a `AdwSwitchRow` for `use_for_lowest_latency`,
  - a `AdwComboRow` of test-URL presets (`gstatic.com/generate_204`, `cp.cloudflare.com/generate_204`, `www.apple.com/library/test/success.html`) that updates the entry row.
- [x] 5.5 Wire toasts via the existing `ToastOverlay` in `app.rs`: "Testing N nodes via real delay…", "Real delay: N/M nodes responded", "Backend does not support real-delay tests (clash_api required)".
- [x] 5.6 Add a global "Test Real Delay (all enabled)" action behind a confirmation dialog when more than 100 nodes are selected.
- [x] 5.7 Verify the page remains responsive while a probe session runs (action is dispatched via `tokio::spawn` into the runtime, with `gtk` updates routed through `relm4::Sender`).

## 6. Documentation and tests

- [x] 6.1 Update `README.md` with a short "Real Delay" section explaining the difference from TCP ping, how to enable it, and the sing-box `with_clash_api` build requirement.
- [x] 6.2 Update `CHANGELOG.md` under a new `Unreleased` section: "Added: Real Delay latency probe via ephemeral isolated backend."
- [x] 6.3 Add an entry to `docs/` (or appropriate location) describing the privacy implication that the configured test URL is reached through the user's proxies.
- [x] 6.4 Run `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`; fix any warnings introduced.
- [ ] 6.5 Manual end-to-end smoke test on a Linux dev machine with sing-box installed: import a real subscription, run "Test Real Delay" on 20 nodes, verify results appear in the UI and persist across restart.

## 7. OpenSpec finalization

- [ ] 7.1 Re-run `openspec validate add-real-delay-latency-test --strict` after implementation and ensure it still passes.
- [ ] 7.2 Open a PR linking the change directory and follow the project's versioning steps (`Cargo.toml`, `pkg/archlinux/PKGBUILD`, `CHANGELOG.md`).
- [ ] 7.3 After merge, archive the change via the OpenSpec archive workflow so the deltas in `specs/` flow into the canonical `openspec/specs/` files.
