# Proposal: inbound-listen-config

## Why

Today the generated v2ray/xray/sing-box configs hard-code the inbound `listen` address to `127.0.0.1`, so the local SOCKS5 and HTTP proxies are only reachable from the same machine. Users who want to share the proxy with other devices on a LAN, a containerised app on a bridge network, or a VM must hand-edit the generated JSON after every regen — which defeats the whole point of the generator.

Separately, while auditing inbound generation we want to make UDP support explicit and verifiable in every generated config, not relying on backend defaults that vary between sing-box and v2ray/xray.

## What Changes

- Add a configurable `listen_address` setting (default `127.0.0.1`) alongside `socks_port` and `http_port` in `AppSettings`, persisted to `settings.toml`.
- Apply that listen address to **both** inbounds (SOCKS5/mixed and HTTP) in the v2ray, xray, and sing-box generators.
- Make UDP support on the SOCKS-capable inbound explicit and verified in tests for every backend:
  - v2ray/xray: keep `"settings": { "udp": true }` on the SOCKS inbound (currently present) and lock it in with a regression test that fails if `udp` is missing or false.
  - sing-box: the `mixed` inbound provides UDP implicitly; add a regression test that asserts the inbound type is `mixed` and that no `udp_disabled: true` is emitted. Do NOT add fake UDP flags that sing-box does not understand.
- Validate the listen address on save: must be a valid IPv4/IPv6 literal (`0.0.0.0`, `::`, `192.168.1.10`, …). Reject hostnames.
- Surface the new field in the Settings page of the UI next to the proxy port fields, with helper text warning that non-loopback addresses expose the proxy to the network.
- Migrate existing `settings.toml` files that lack the field by filling in the default on load.

## Capabilities

### Modified Capabilities
- `config-generator`: Inbound `listen` address is configurable instead of hard-coded; UDP behaviour on the SOCKS-capable inbound is explicit and tested for every backend.
- `app-persistence`: `AppSettings` gains a `listen_address` field with default `127.0.0.1` and backward-compatible deserialization.

## Impact

- Modified models: `AppSettings` in `crates/core/src/models/settings.rs` (new field + default + validation).
- Modified generators: `crates/core/src/config/v2ray.rs` and `crates/core/src/config/singbox.rs` (xray reuses v2ray).
- Modified UI: settings page in `crates/ui/src/settings.rs`.
- New tests: per-backend tests asserting `listen` reflects the setting and UDP is enabled on the SOCKS-capable inbound.
- No protocol or process-lifecycle behaviour changes; binary backends are launched the same way.
- Security note: allowing non-loopback listen addresses widens the attack surface. The setting defaults to `127.0.0.1` and the UI shows a warning when the user changes it.
