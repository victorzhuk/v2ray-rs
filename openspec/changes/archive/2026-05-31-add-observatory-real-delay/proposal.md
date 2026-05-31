# Proposal: Observatory Real Delay for xray and v2ray

## Why

The first Real Delay implementation supports sing-box by driving its Clash-compatible delay API. That gives users an end-to-end latency signal for sing-box, but users running xray or v2ray still only get the cheap TCP probe. TCP latency is useful for background freshness, but it does not verify the proxy protocol handshake, TLS/REALITY negotiation, or the proxy server's own egress path.

Xray-core and v2fly/v2ray-core already expose backend-native observatory services that report per-outbound health and delay. We should use those APIs rather than implementing proxy protocols in Rust or pretending TCP ping is equivalent to a real proxy request.

## What Changes

- Enable Real Delay for xray by generating an ephemeral probe config with `burstObservatory` and querying `xray.core.app.observatory.command.ObservatoryService/GetOutboundStatus` over gRPC.
- Enable Real Delay for v2ray builds that expose the v2fly observatory service by generating an equivalent probe config and querying `v2ray.core.app.observatory.command.ObservatoryService/GetOutboundStatus` over gRPC.
- Keep v2ray/xray probing isolated in the existing short-lived `ProbeRunner`; never modify the user-facing backend process.
- Add minimal protobuf/gRPC bindings for observatory status only. The bindings SHALL cover only request/response messages needed to read `outbound_tag` and `delay` values.
- Re-enable xray/v2ray Real Delay capability gating only after the gRPC path works. Unsupported binaries SHALL continue to show a disabled UI action with a clear explanation.
- Keep sing-box behavior unchanged.

## Capabilities

### New Capabilities
- `observatory-real-delay`: xray/v2ray Real Delay measurement through backend-native Observatory/BurstObservatory APIs.

### Modified Capabilities
- `backend-detection`: backend capability reporting distinguishes sing-box Clash API support from xray/v2ray observatory support.

## Impact

- **Crates affected**:
  - `v2ray-rs-core`: probe config generation for xray/v2ray; backend capability helpers.
  - `v2ray-rs-subscription`: gRPC observatory client and xray/v2ray orchestration branch in `measure_real_delay`.
  - `v2ray-rs-ui`: Real Delay action becomes enabled for supported xray/v2ray binaries after capability detection.
- **Dependencies**: likely `tonic`, `prost`, and a build-time protobuf strategy (`tonic-build`/`prost-build` or checked-in generated minimal bindings). Avoid any proxy-protocol dependency.
- **External behavior**: xray/v2ray users can run the same Real Delay action that currently works for sing-box. Legacy v2ray builds without Observatory remain unsupported.
- **Risk**: Observatory is timer-driven. The app must poll with a bounded timeout and tolerate partial results.
