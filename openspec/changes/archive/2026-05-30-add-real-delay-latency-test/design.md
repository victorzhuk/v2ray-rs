# Design: Real Delay Latency Test

## Context

v2ray-rs measures node "latency" today via [`tcp_ping`](file:///home/zhuk/Projects/own/v2ray-rs/crates/subscription/src/ping.rs#L22-L30):
a single TCP `connect` to `host:port` with a 5 s timeout. The result is the L4 RTT to the
proxy's edge, not the latency of the proxy itself.

Every comparable project (v2rayN, Mihomo/Clash, sing-box, Nekobox, v2rayNG, Hiddify)
exposes a second probe — **Real Delay** — that sends an HTTP request through the proxy
to a known endpoint and reports the wall-clock RTT of the full chain. The canonical
reference implementation is sing-box's [`common/urltest/urltest.go`](https://github.com/SagerNet/sing-box/blob/dev-next/common/urltest/urltest.go):
it dials through the outbound, issues `HEAD https://www.gstatic.com/generate_204`, and
times the request. Mihomo/Clash exposes the same logic via the HTTP API
(`GET /proxies/{name}/delay?url=...&timeout=...`). xray-core exposes it via its
`Observatory` / `BurstObservatory` outbound and the gRPC stats API.

Our project's foundational constraint (from CLAUDE.md) is that we never implement proxy
protocols ourselves; the installed backend binary owns all wire-format work. That rules
out a third-party Rust implementation of VLESS/VMess/Trojan/SS clients just to do this
test, and it also rules out shoving the test into the main backend process (would
interfere with the user's live traffic, requires restart per probe, breaks the
process-lifecycle contract).

The remaining design space is "spawn an isolated, short-lived backend instance to perform
the probe, then shut it down". This is what we design here.

## Goals / Non-Goals

**Goals:**
- Provide an honest, end-to-end "Real Delay" measurement per node.
- Keep the existing `tcp_ping` for cheap/background use; do **not** replace it.
- Do not interrupt or modify the user-facing backend process.
- Do not implement any proxy protocol in Rust.
- Persist Real Delay results alongside TCP samples so the UI can show both.
- Support sing-box and xray. Degrade gracefully (TCP-only) when the installed backend
  lacks the required feature (e.g. sing-box built without `with_clash_api`).
- Configurable test URL and timeout (sensible defaults).

**Non-Goals:**
- Bandwidth / download-speed testing. (Different feature; significant extra design.)
- Streaming-platform unlock detection.
- Packet loss / jitter measurement beyond a single sample.
- Auto-running Real Delay on the 10-minute background tick. Real Delay is expensive
  (spawns a process, drives traffic through every node); we keep background TCP-only
  and let the user trigger Real Delay on demand.
- HTTP/2 PING-frame probe optimization (sing-box upstream issue #1494). Possible future
  optimization; out of scope here.

## Decisions

### D1. Drive the probe through an ephemeral backend, not via in-process Rust

**Decision:** Spawn a second backend instance with a generated probe config, query it
via the backend's own API for results, then shut it down.

**Alternatives considered:**
- *Implement protocol clients in Rust* (e.g. via `shadowsocks-rust`, a VLESS client crate).
  Rejected: violates the project's no-protocol-logic invariant; multiplies maintenance.
- *Reuse the main backend process and add a temporary outbound.* Rejected: requires hot
  config reload (not uniformly supported), interferes with live traffic, and shape of
  the user's running config becomes a test variable.
- *Add a SOCKS/HTTP inbound to the main backend and drive HTTP through it.* Rejected
  for the same live-traffic-interference reasons; also tests only the currently active
  outbound, not arbitrary nodes.

### D2. Bulk probe per session using the backend's native test surface

**Decision:** Test all selected nodes in a single ephemeral backend instance:

- **sing-box (≥1.4 with `with_clash_api`)**: generate a config with all candidate nodes
  as outbounds plus a `clash_api` experimental block on a random localhost port. For
  each node, call `GET http://127.0.0.1:<api_port>/proxies/<tag>/delay?url=<test_url>&timeout=<ms>`
  in parallel. The Clash API returns `{ "delay": <ms> }` or HTTP 408/504 on failure.
  This is exactly how Mihomo, Nekobox, and other GUIs talk to sing-box for this purpose.
- **xray-core (≥1.5)**: generate a config with a `burstObservatory` block listing the
  candidate node tags and a `routing.balancers` selector group, plus a Stats service on
  a random localhost port for the gRPC API. After ~`probeInterval` × N seconds, fetch
  the observation report and read `delay` (HTTP probe RTT) per outbound tag.
  Alternative: use the older `observatory` block (deprecated) for v2ray-core legacy.
- **v2ray-core legacy**: best-effort `observatory` if present, else feature unsupported.

**Why bulk:** spawning one process for every node is too slow (~50–200 ms per spawn ×
hundreds of nodes); spawning one process per test session amortizes startup cost.

**Why the backend's own API:** the backend already knows how to dial through itself,
including transport, TLS, REALITY, multiplex, etc. Re-implementing the dispatch in Rust
would be wrong and brittle.

### D3. Extract a reusable "isolated backend" helper from `ProcessManager`

`ProcessManager` today is tightly coupled to a single, long-lived, user-facing backend
process: PID file under the user's data dir, broadcast events to the tray, crash recovery,
log capture into the main UI buffer. None of that fits an ephemeral probe instance.

**Decision:** Add `ProbeRunner` in `v2ray-rs-process` (separate from `ProcessManager`):

```rust
pub struct ProbeRunner {
    binary: PathBuf,
    config_path: PathBuf,   // temp file
    api_port: u16,          // chosen by caller
    child: Option<Child>,
}

impl ProbeRunner {
    pub async fn start(&mut self) -> Result<(), ProbeError>;
    pub async fn wait_ready(&mut self, timeout: Duration) -> Result<(), ProbeError>;
    pub async fn stop(&mut self) -> Result<(), ProbeError>;  // SIGTERM, then SIGKILL
}
```

It does **not** publish state events, does **not** auto-restart on crash, does **not**
write a PID file in `~/.local/share`, and uses `tempfile` for the config and a private
log sink (kept only for diagnostics on error). On `Drop`, it kills the child to prevent
orphaned probe processes.

### D4. Config generation lives next to existing generators

Add to `crates/core/src/config/`:

- `probe.rs` — module with a `ProbeConfigGenerator` trait and
  `probe_generator_for(backend) -> Box<dyn ProbeConfigGenerator>`.
- Per-backend implementations (`xray_probe.rs`, `singbox_probe.rs`) reuse the existing
  outbound-emission helpers so the probe sees the **same** outbound config as a normal
  run (transport, TLS settings, etc.).

The probe config:
- has **no** user inbounds (no SOCKS/HTTP open to the world);
- has no routing rules (`final`/`finalOutbound` = the node under test or, for bulk,
  the API selects per request);
- has a Clash API listener (sing-box) or gRPC API (xray) bound to `127.0.0.1:<port>`
  with no auth — the port is ephemeral and bound to loopback only.

### D5. Caller orchestration in `v2ray-rs-subscription`

New file `crates/subscription/src/real_delay.rs`:

```rust
pub struct RealDelayConfig { pub test_url: String, pub timeout: Duration }

pub async fn measure_real_delay(
    backend: &BackendInfo,
    nodes: &[SubscriptionNode],
    cfg: &RealDelayConfig,
    paths: &AppPaths,
) -> Vec<Option<u64>>;
```

Steps:
1. Pick a free localhost port (`TcpListener::bind("127.0.0.1:0")` + drop).
2. Generate probe config via the per-backend generator into a `NamedTempFile`.
3. Spawn `ProbeRunner`, wait for the API port to accept connections (≤2 s).
4. For sing-box: issue parallel `GET /proxies/{tag}/delay?...` requests via the existing
   `reqwest::Client`. For xray: trigger observation, poll the gRPC stats endpoint, then
   read results.
5. Map outbound-tag → node index → `Option<u64>` ms.
6. On `Drop`/exit: stop the runner, temp files auto-clean.

Concurrency limit: at most 1 ephemeral backend at a time per app (a global
`tokio::sync::Mutex`), to bound resource use and avoid port conflicts.

### D6. Persistence

Extend [`SubscriptionNode`](file:///home/zhuk/Projects/own/v2ray-rs/crates/core/src/models/subscription.rs)
with `last_real_delay_ms: Option<u64>` (serde default `None`).

Extend `latency_snapshot.json` payload from
```json
{"sub_id":"...", "node_index":0, "tcp_ms":42}
```
to
```json
{"sub_id":"...", "node_index":0, "tcp_ms":42, "real_ms":null}
```
Backward-compatible: missing `real_ms` deserializes as `None` via `#[serde(default)]`.

Extend `AppSettings` with:

```rust
pub struct RealDelaySettings {
    pub enabled: bool,                 // default true (UI offers it)
    pub test_url: String,              // default "https://www.gstatic.com/generate_204"
    pub timeout_ms: u32,               // default 5000
    pub use_for_lowest_latency: bool,  // default false; opt-in for auto-resolve
}
```

### D7. UI

- Subscriptions page: existing "Test latency" button stays. Add overflow-menu / right-click
  action "Test real delay" (per subscription and per selected nodes). Two-line latency
  badge or `tcp 38 ms · real 412 ms`.
- Settings dialog: add Real Delay section (test URL, timeout, "Use Real Delay for Lowest
  Latency strategy" toggle).
- Toasts for: "real-delay test started", "backend does not support clash_api / observatory"
  (with link to docs), "completed: N/M nodes responded".

### D8. Auto-resolve integration

`ConnectionPlanner` already reads `last_latency_ms`. When `use_for_lowest_latency` is
true AND a node has `last_real_delay_ms`, sort by that field; fall back to TCP sample;
unknown values placed last (unchanged rule).

## Risks / Trade-offs

- **[Backend feature gating]** sing-box may be built without `with_clash_api`; some
  distros (Arch `sing-box-core`, Alpine) do enable it, but not all. → Mitigation: at
  runtime, probe `clash_api` by attempting to start the ephemeral instance with a
  minimal config and `GET /version` on the API; if it fails, surface a toast and
  disable Real Delay actions for sing-box on this machine until the user reinstalls.
  Document the build flag requirement in README.

- **[Process spawn cost]** Each probe session pays ~50–200 ms for backend startup. →
  Mitigation: bulk per session (D2). Add an optional "keep-alive" mode later if users
  ask for sub-second repeat probes (out of scope here).

- **[Port collisions]** Race between `bind(0)` returning a port and the backend
  binding it. → Mitigation: use a small retry loop (3 attempts) on ephemeral bind
  failure; in practice the window is microseconds and the loopback range is huge.

- **[ETXTBSY on overlayfs/containers]** Already handled by `ProcessManager` for the
  main process. → Mitigation: copy the same retry logic into `ProbeRunner` (extract a
  shared `spawn_with_etxtbsy_retry` helper in `v2ray-rs-process`).

- **[Test URL censorship]** `gstatic.com` is reachable in most networks but blocked or
  rate-limited in some. → Mitigation: configurable URL, ship a presets list
  (`gstatic.com/generate_204`, `cp.cloudflare.com/generate_204`,
  `www.apple.com/library/test/success.html`). UI shows the dropdown.

- **[Privacy]** The test URL is reached **through** the user's proxy nodes. This is the
  intended behaviour (mirroring real workload) but should be documented so a user
  reviewing traffic logs at the proxy server isn't surprised.

- **[Orphaned probe processes]** If v2ray-rs is killed mid-probe with `SIGKILL`, the
  child backend may survive. → Mitigation: on next startup, `ProbeRunner` is never used
  for the main process; the probe PID is not persisted, so we accept this rare leak.
  Document and rely on `prctl(PR_SET_PDEATHSIG)` where available (Linux-only) so the
  kernel cleans up the probe when its parent dies.

- **[xray Observatory complexity]** xray's BurstObservatory operates on a timer; getting
  one-shot results requires waiting at least one probe interval. → Mitigation: set
  `probeInterval` to a small value (`500ms`) and `probeUrl` to the configured test URL,
  then wait `timeout + 1s`, then read results. Acceptable for an on-demand action.

## Migration Plan

1. Ship behind a `RealDelaySettings { enabled: true, ... }` field; legacy settings load
   with the default. No user-visible change until they invoke the new action.
2. Snapshot file: forward-compatible (new field is `Option`, default `None`); reading an
   old snapshot just leaves `last_real_delay_ms` as `None`. No down-migration needed —
   older v2ray-rs versions ignore the new field on load.
3. Capability sync: the changed/added capabilities are appended via OpenSpec deltas; no
   spec is removed.

## Open Questions

- Should we expose Real Delay results in the tray's per-node tooltip, or keep the tray
  showing TCP only? (Default: keep TCP only in tray; Real Delay lives in the main UI.)
- For xray, should we use `observatory` (simpler, deprecated) or `burstObservatory`
  (modern, recommended)? Default: `burstObservatory` when xray ≥ 1.8, else fall back.
- Do we want a "Test Real Delay (all enabled nodes)" shortcut from the main window, or
  only per-subscription / per-selection? (Plan: both; the global action is hidden behind
  a confirmation when > 100 nodes are selected.)
