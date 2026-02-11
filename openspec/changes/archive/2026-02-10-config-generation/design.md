## Context

The app generates JSON config files that the backend CLI tools consume. v2ray and xray share a similar config schema but sing-box uses a different format. Config must be regenerated whenever subscriptions or routing rules change.

## Goals / Non-Goals

**Goals:**
- Generate valid, complete JSON configs for v2ray, xray, and sing-box
- Include inbound (local SOCKS5/HTTP proxy), outbound (proxy nodes), and routing sections
- Trigger regeneration reactively on data changes
- Write configs atomically to prevent corruption

**Non-Goals:**
- No config schema validation beyond structural correctness
- No support for editing raw JSON in the UI (future enhancement)

## Decisions

1. **Generator trait**: Define a `ConfigGenerator` trait with method `generate(nodes: &[ProxyNode], rules: &[RoutingRule], settings: &AppSettings) -> Result<serde_json::Value>`. Implement per backend: `V2rayGenerator`, `XrayGenerator`, `SingboxGenerator`.

2. **Config structure**: Build configs as `serde_json::Value` trees. This avoids defining separate Rust types for each backend's full config schema (which are large and change across versions). Use builder methods for each config section (inbounds, outbounds, routing).

3. **Reactive regeneration**: Use a message/event channel. When subscription or routing data changes, emit an event. Config module listens and regenerates. This decouples data changes from config writing.

4. **Output path**: Default to `$XDG_DATA_HOME/v2ray-rs/generated/<backend>.json`. User-configurable.

## Risks / Trade-offs

- [serde_json::Value vs typed structs] Lose compile-time guarantees for config structure → Mitigated by integration tests validating output against real backends
- [Backend version differences] Config schema differs across backend versions → Target latest stable versions, document supported versions
