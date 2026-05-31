# Tasks: Observatory Real Delay for xray and v2ray

## 1. Protobuf and gRPC client plumbing

- [x] 1.1 Add hand-written minimal `prost::Message` structs for xray and v2fly/v2ray-core observatory messages: `GetOutboundStatusRequest`, `GetOutboundStatusResponse`, `ObservationResult`, `OutboundStatus`, and `HealthPingMeasurementResult` only if decoding requires it.
- [x] 1.2 Call `GetOutboundStatus` with `tonic::client::Grpc::unary` and explicit service paths; avoid `tonic-build`, `prost-build`, generated clients, and system `protoc` unless hand-written structs prove insufficient.
- [x] 1.3 Add `tonic`/`prost` dependencies in workspace-managed form, only to crates that need them.
- [x] 1.4 Implement a backend-neutral `ObservatoryStatus { outbound_tag, delay_ms, alive, last_error }` adapter in `crates/subscription`.
- [x] 1.5 Implement `query_xray_observatory(api_port)` against `xray.core.app.observatory.command.ObservatoryService/GetOutboundStatus`.
- [x] 1.6 Implement `query_v2ray_observatory(api_port)` against `v2ray.core.app.observatory.command.ObservatoryService/GetOutboundStatus`.
- [x] 1.7 Unit-test protobuf mapping with mock tonic servers or encoded-response fixtures for xray and v2ray; cover alive delay, dead outbound, missing tag, malformed response, and transport errors.

## 2. Probe config generation

- [x] 2.1 Re-enable `XrayProbeGenerator` in `probe_generator_for(BackendType::Xray)` only after the gRPC query path exists.
- [x] 2.2 Add `V2rayProbeGenerator` and return it from `probe_generator_for(BackendType::V2ray)` when v2ray observatory support is targeted.
- [x] 2.3 Correct xray observatory JSON casing and shape: legacy `observatory` uses `probeURL`; `burstObservatory.pingConfig` uses `destination`, short `interval`, configured `timeout`, `sampling = 1`, and `httpMethod = "HEAD"` where supported.
- [x] 2.4 Emit v2ray-compatible observatory config using v2fly's expected JSON shape: `burstObservatory` with `destination`, short `interval`, configured `timeout`, `sampling = 1`, and no `httpMethod` unless the target schema supports it; optionally fall back to legacy `observatory` with the probed `probe-` selector.
- [x] 2.5 Ensure generated xray/v2ray probe configs include only loopback API inbounds and no user-facing SOCKS/HTTP inbounds.
- [x] 2.6 Ensure API inbound routing targets the API outbound tag and does not route test traffic away from the probed proxy outbounds.
- [x] 2.7 Add config-shape tests for xray and v2ray probes covering representative VLESS, VMess, Trojan, and Shadowsocks nodes.

## 3. Real Delay orchestration

- [x] 3.1 Refactor `measure_real_delay` to dispatch by backend: sing-box keeps the existing Clash API path; xray/v2ray use the observatory gRPC path.
- [x] 3.2 Add `observatory_delays(backend, api_port, count, timeout_ms)` that polls until all expected `probe-<idx>` tags return successful delays or the deadline expires.
- [x] 3.3 Map `OutboundStatus.outbound_tag` back to node index by parsing the `probe-<idx>` suffix; ignore unknown tags safely.
- [x] 3.4 Treat `alive = false`, missing `delay`, zero/negative delay, and per-tag `last_error_reason` as `None` for that node while preserving other successful results.
- [x] 3.5 Keep the process-wide `REAL_DELAY_LOCK` around xray/v2ray sessions so only one ephemeral backend runs at a time.
- [x] 3.6 Always call `ProbeRunner::stop().await` after xray/v2ray success, timeout, gRPC error, or early process exit.
- [x] 3.7 Add orchestration tests with mocked observatory clients for full success, partial success, timeout, unknown tags, and probe-backend crash.

## 4. Capability gating and UI

- [x] 4.1 Replace purely boolean Real Delay availability with a session capability state: `Unsupported`, `PotentiallySupported`, and `Supported`.
- [x] 4.2 Update UI gating so xray and v2ray start as potentially supported, with tooltip text explaining that ObservatoryService availability is checked when the probe runs.
- [x] 4.3 Preserve disabled behavior for known unsupported backends/builds; use backend-specific tooltips/diagnostics (`sing-box with Clash API`, `xray ObservatoryService`, `v2ray ObservatoryService`).
- [x] 4.4 Reset cached Real Delay capability when backend type or binary path changes.
- [x] 4.5 Sync capability changes into `SubscriptionsPage` when backend settings change, so switching between sing-box/xray/v2ray immediately updates Real Delay menu sensitivity.
- [x] 4.6 Ensure Real Delay result persistence, badges, sorting, and Lowest Latency integration reuse the existing `last_real_delay_ms` path without backend-specific UI branching.

## 5. Documentation and validation

- [x] 5.1 Update `README.md` to say Real Delay supports sing-box with Clash API plus xray/v2ray builds with ObservatoryService.
- [x] 5.2 Update `docs/real-delay-privacy.md` if needed to mention xray/v2ray observatory probes still contact the configured URL through each proxy node.
- [x] 5.3 Update `CHANGELOG.md` under `[Unreleased]` with xray/v2ray Real Delay support.
- [x] 5.4 Add ignored integration tests that run a real installed `xray` and `v2ray` binary when present, start a probe config, query observatory status, and verify graceful shutdown.
- [x] 5.5 Run `cargo fmt`.
- [x] 5.6 Run `cargo test --workspace`.
- [x] 5.7 Run `cargo clippy --workspace -- -D warnings`.
- [x] 5.8 Run `openspec validate add-observatory-real-delay --strict` and fix all validation findings.
