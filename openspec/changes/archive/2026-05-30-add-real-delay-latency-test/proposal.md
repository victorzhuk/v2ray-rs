# Proposal: Real Delay Latency Test

## Why

The current latency probe (`tcp_ping` in `crates/subscription/src/ping.rs`) only measures the time
to complete a TCP handshake to the proxy server's IP/port. This number is misleading for users
because it does NOT verify that:

- the proxy protocol (VLESS/VMess/Trojan/Shadowsocks) actually negotiates successfully,
- the TLS/REALITY handshake completes,
- the server's outbound to the real internet works,
- the server is unblocked from the user's network end-to-end.

A node with `30 ms` TCP ping can be completely unusable (TLS reset, throttled, blocked egress),
and a node with `250 ms` TCP ping can be a perfectly working proxy. Every mature proxy GUI
(v2rayN, Nekobox, Clash/Mihomo, sing-box, Hiddify) ships a second test called **"Real Delay"**
or **"URL Test"** that performs an HTTP request to a known endpoint **through** the proxy,
and reports the wall-clock time of the full chain (TCP + TLS + protocol handshake + HTTP RTT).

We want the same honest measurement, while keeping the project's invariant that **all protocol
work is delegated to the installed backend binary** (`v2ray` / `xray` / `sing-box`) and
v2ray-rs never implements proxy protocols itself.

## What Changes

- Add a second latency mode, **Real Delay**, alongside the existing TCP ping.
- Real Delay is performed by spawning an **ephemeral, isolated backend instance** (separate
  from the user-facing proxy process) with a generated config that:
  - exposes a Clash-compatible API (`clash_api` for sing-box) or uses the Observatory
    feature (for xray), and
  - contains the nodes under test as outbounds.
- Trigger the test via the backend's own delay endpoint:
  - **sing-box**: `GET /proxies/{tag}/delay?url=<test_url>&timeout=<ms>` on the Clash API.
  - **xray**: enable `observatory` / `burstObservatory` outbound and read results from the
    gRPC stats / observation API.
  - **v2ray (legacy)**: same observatory API as xray when available; otherwise fall back to
    "not supported" and keep TCP ping for v2ray users.
- The test URL and per-probe timeout are configurable in `AppSettings`. Defaults follow
  community convention: `https://www.gstatic.com/generate_204`, 5 s timeout.
- Real Delay is **opt-in per action**: the existing "Test Latency" button keeps doing TCP
  ping (fast, no extra process); a new "Test Real Delay" action runs the new probe.
- The scheduled 10-minute background refresh and the startup hydration keep using **TCP
  ping** (cheap, no second backend process). Real Delay is on-demand only.
- Per-node latency storage gains a second field so both samples coexist in `latency_snapshot.json`
  and in the subscriptions UI.
- The Subscriptions UI shows both numbers (e.g. `tcp 38 ms · real 412 ms`) and lets the user
  sort by either.
- The auto-resolve "Lowest Latency" strategy gains a settings toggle: order by TCP ping
  (default, current behaviour) or by Real Delay when a sample exists.
- **BREAKING**: `latency_snapshot.json` schema gains a new `real_delay_ms` field. A migration
  reads old snapshots (with only `last_latency_ms`) and treats missing real-delay values as
  unknown without erroring.

## Capabilities

### New Capabilities
- `real-delay-latency-test`: orchestrating an ephemeral backend instance, generating its
  config, invoking the backend's delay/observatory API, parsing results, and persisting them
  alongside TCP samples.

### Modified Capabilities
- `background-latency-testing`: clarify that the scheduled and startup paths remain
  TCP-only, and that Real Delay is a separate on-demand path.
- `connection-auto-resolve`: extend the Lowest Latency strategy to optionally rank by Real
  Delay when both samples exist.
- `app-persistence`: `latency_snapshot.json` schema gains a `real_delay_ms` field with a
  forward-compatible migration.
- `subscription-import`: `SubscriptionNode` gains a `last_real_delay_ms: Option<u64>` field
  surfaced in the UI.

## Impact

- **Crates affected**:
  - `v2ray-rs-subscription`: new `real_delay` module that drives the ephemeral-backend probe;
    `ping.rs` stays as-is.
  - `v2ray-rs-process`: factor out a reusable "spawn isolated backend" helper from
    `ProcessManager` so the probe can launch a second backend without polluting the user's
    main process state (separate PID file, separate log buffer, no crash-recovery, no tray
    events).
  - `v2ray-rs-core`:
    - `models/subscription.rs`: add `last_real_delay_ms`.
    - `models/settings.rs`: add `real_delay: RealDelayConfig { test_url, timeout_ms,
      enabled_for_lowest_latency }`.
    - `persistence.rs`: migrate `latency_snapshot.json`.
    - `config/`: add a generator for the ephemeral probe config (Clash-API-enabled
      sing-box and observatory-enabled xray variants).
    - `resolve.rs`: `ConnectionPlanner` reads the new field when configured.
  - `v2ray-rs-ui`:
    - `subscriptions.rs`: new "Test Real Delay" menu action, second column / inline badge,
      sort-by-real-delay option.
    - `settings.rs`: new section for the test URL and timeout.
- **Dependencies**: add a lightweight HTTP client only if not already present (we already
  have `reqwest` with `rustls-tls` in the subscription crate — reuse it). No new protocol
  dependencies.
- **External**: requires the installed backend to support either the Clash API
  (sing-box ≥ 1.0 built with `with_clash_api`) or Observatory (xray-core, v2ray-core ≥ 5).
  Detect at runtime; degrade gracefully with a user-visible toast when unsupported.
- **Performance**: launching a second backend costs ~50–200 ms of startup overhead per
  test session. We test all selected nodes in one ephemeral instance (`urltest` /
  observatory probes them in parallel), so per-node cost approaches the test URL's RTT.
- **Privacy**: the test URL is contacted **through** the user's proxy nodes, exactly as a
  real workload would be. The user can change the URL if `gstatic.com` is undesirable in
  their region.
