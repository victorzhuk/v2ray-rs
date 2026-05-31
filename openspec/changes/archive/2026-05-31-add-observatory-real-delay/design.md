# Design: Observatory Real Delay for xray and v2ray

## Context

The existing Real Delay path uses a short-lived backend instance and delegates all proxy protocol work to the installed backend. sing-box is straightforward because its Clash API exposes `GET /proxies/{tag}/delay`. Xray-core and v2fly/v2ray-core do not expose an equivalent HTTP endpoint, but both expose a gRPC `ObservatoryService` that returns an `ObservationResult` containing per-outbound `OutboundStatus` records.

Relevant upstream service shapes:

- Xray: `xray.core.app.observatory.command.ObservatoryService/GetOutboundStatus` with an empty request and `ObservationResult.status[]` response entries.
- V2Ray: `v2ray.core.app.observatory.command.ObservatoryService/GetOutboundStatus` with optional `Tag` in the request and the same `ObservationResult.status[]` shape.
- Each `OutboundStatus` includes `alive`, `delay`, `last_error_reason`, and `outbound_tag`.

The bundled `xray api` and `v2ray api` commands do not currently expose observatory status directly, so shelling out to the backend CLI is not enough. We need a tiny Rust gRPC client.

## Goals / Non-Goals

**Goals:**
- Add xray and v2ray Real Delay without implementing proxy protocols in Rust.
- Reuse the existing ephemeral probe process and Real Delay persistence/UI model.
- Poll the backend-native observatory service and map `outbound_tag` values back to `probe-<idx>` nodes.
- Keep unsupported builds disabled with clear user-facing diagnostics.

**Non-Goals:**
- Replacing TCP latency probes.
- Adding speed tests, unlock tests, jitter, or packet-loss metrics.
- Embedding xray/v2ray core libraries.
- Using the user's live backend process for probing.

## Decisions

### D1. Use gRPC ObservatoryService, not CLI scraping

The backend CLIs expose stats, balancer info, and mutation APIs, but not raw observatory delay status. Calling the gRPC service directly is the most stable way to read `OutboundStatus.delay` without adding Go helper binaries or parsing human-readable command output.

### D2. Hand-write minimal protobuf messages

Use hand-written `prost::Message` structs for only the observatory messages used by this feature:

- Xray package/service names.
- V2Ray package/service names.
- `GetOutboundStatusRequest`.
- `GetOutboundStatusResponse`.
- `ObservationResult`.
- `OutboundStatus`.
- `HealthPingMeasurementResult` only if decoding requires it.

Call gRPC manually with `tonic::client::Grpc::unary` and explicit service paths:

- `/xray.core.app.observatory.command.ObservatoryService/GetOutboundStatus`
- `/v2ray.core.app.observatory.command.ObservatoryService/GetOutboundStatus`

This avoids a system `protoc`, checked-in generated code, and a broad dependency on upstream API surfaces. Use generated bindings only if hand-written structs prove insufficient.

### D3. Keep separate client modules per backend family

Xray and V2Ray service package names differ even though the response shape is nearly identical. Keep a small adapter layer:

```rust
pub async fn query_xray_observatory(port: u16) -> Result<Vec<ObservatoryStatus>, ObservatoryError>;
pub async fn query_v2ray_observatory(port: u16) -> Result<Vec<ObservatoryStatus>, ObservatoryError>;

pub struct ObservatoryStatus {
    pub outbound_tag: String,
    pub delay_ms: Option<u64>,
    pub alive: bool,
    pub last_error: Option<String>,
}
```

The rest of `measure_real_delay` should not care which protobuf package produced the data.

### D4. Use one-shot-friendly burst observatory configs

For xray, emit `burstObservatory` with:

- `subjectSelector: ["probe-"]`.
- `pingConfig.destination` set to the configured test URL.
- `pingConfig.interval` short enough for on-demand probing.
- `pingConfig.timeout` set from `RealDelaySettings::timeout_ms`.
- `pingConfig.sampling` set to `1`.
- `pingConfig.httpMethod` set to `HEAD` where supported.

For legacy xray `observatory`, use `probeURL` casing.

For v2ray, emit the v2fly-compatible `burstObservatory` shape. v2fly's JSON config uses `sampling`, and its current `HealthCheckSettings` does not expose `httpMethod`; omit backend-unsupported fields. If a build rejects `burstObservatory`, optionally fall back to legacy `observatory` only if the generated config validates and the observatory service becomes reachable.

### D5. Poll results with bounded timeout

After `ProbeRunner::wait_ready`, poll `GetOutboundStatus` until one of these conditions occurs:

- all expected `probe-<idx>` tags have a successful positive `delay`,
- the per-session deadline expires (`timeout_ms + 1500 ms`, minimum guard of 2 seconds),
- the probe process exits early.

Partial results are valid: record successful tags and leave missing/error tags as `None`.

### D6. Capability gating is stateful

Backend type is not enough to know Real Delay availability. Model xray/v2ray as potentially supported until a probe confirms or rejects the required service:

```rust
enum RealDelayCapability {
    Unsupported { reason: String },
    PotentiallySupported { requirement: &'static str },
    Supported,
}
```

The UI may enable Real Delay for `PotentiallySupported` backends and explain that support will be checked on run. A failed observatory probe records an in-session unsupported reason. Changing the backend type or binary path resets the state to potential support.

### D7. Local HTTP inbounds remain a fallback design

An alternate design is to generate one loopback HTTP inbound per probed node, route each inbound to one `probe-<idx>` outbound, and have v2ray-rs issue a normal HTTP request through that local proxy. The backend still performs all proxy protocol handshakes, so this does not violate the no-protocol-logic invariant.

This change keeps the backend-native ObservatoryService path because it matches xray/v2ray's health-check model and the existing sing-box backend-native design. The local-inbound design remains a fallback if observatory polling proves too brittle.

## Risks / Trade-offs

- **Protobuf drift:** Xray and V2Ray package names differ and may evolve. Mitigate by keeping the generated surface minimal and adding fixture tests for encoded/decoded status responses.
- **Timer-driven observatory:** Results may not appear immediately. Mitigate with polling and clear partial-result handling.
- **Manual protobuf drift:** hand-written `prost` structs depend on upstream field numbers. Mitigate with fixture decode tests and a small documented message surface.
- **Config compatibility:** xray and v2ray differ in JSON key casing and observatory variants. Mitigate with backend-specific probe generator tests and ignored real-binary integration tests.

## Migration Plan

No persisted data migration is required. Existing Real Delay settings and latency snapshot fields are reused. This change only broadens which backend types can produce `last_real_delay_ms` samples.

## Open Questions

- Should capability detection proactively start a tiny observatory config during Preferences backend validation, or should detection remain lazy at first Real Delay run?
- Should v2ray support target only v2fly v5 builds, or attempt legacy v4-compatible observatory configs as best effort?
