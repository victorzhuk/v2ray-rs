## Why

Node transport settings are emitted differently per backend without telling the user. A WebSocket node with custom headers and a separate host keeps the host on sing-box but loses it on v2ray and xray, so the connection goes out with the wrong `Host`. The v2ray backend emits REALITY `realitySettings` and `network: "xhttp"` as if it supported them, while sing-box refuses an unsupported transport with an error naming the node; on v2ray the failure surfaces later as a backend startup error or a silently different connection. sing-box drops gRPC multi mode and REALITY spider X, yet the manual node editor offers both without any hint that one backend ignores them.

## What Changes

- v2ray and xray WebSocket settings merge the node's host into the headers as `Host` when the headers carry no `Host` of their own, matching sing-box; xray then moves it to its `host` field as today.
- The v2ray backend refuses a node that uses REALITY or the XHTTP transport, failing config generation for that candidate with an error naming the node and the feature. The v2ray Real Delay probe skips such nodes instead of failing the batch, like the sing-box probe does for XHTTP.
- The manual node editor marks gRPC multi mode and REALITY spider X as not used by sing-box. Generation output for sing-box does not change.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `config-generator`: new requirements for WebSocket host parity and for the v2ray backend refusing xray-only node features.
- `ui-lists`: new requirement for backend notes on node editor fields sing-box ignores.

## Impact

- `crates/core/src/config/v2ray.rs` — `build_ws_settings`, `V2rayGenerator::generate` feature check.
- `crates/core/src/config/mod.rs` — `ConfigError` variant for an unsupported node feature.
- `crates/core/src/config/probe.rs` — `V2rayProbeGenerator` skips refused nodes.
- `crates/ui/src/nodes.rs` — row titles/subtitles for `grpc_multi_mode` and `spider_x`.
