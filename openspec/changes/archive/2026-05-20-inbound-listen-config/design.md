# Design: inbound-listen-config

## Context

Inbound generation today lives in two places:

- `crates/core/src/config/v2ray.rs::build_inbounds` (also reused by xray via `generate_v2ray_family_config`)
- `crates/core/src/config/singbox.rs::build_inbounds`

Both functions hard-code `"listen": "127.0.0.1"`. The v2ray/xray SOCKS inbound already declares `"settings": { "udp": true }`. The sing-box `mixed` inbound has no explicit UDP toggle because `mixed` supports UDP by definition in sing-box.

Settings live in `AppSettings` (`crates/core/src/models/settings.rs`), serialised to TOML through `serde`. The struct currently exposes `socks_port: u16` and `http_port: u16` with defaults `1080` / `1081`.

## Decision

### Data model

Add a single `listen_address: String` field to `AppSettings`:

```rust
#[serde(default = "default_listen_address")]
pub listen_address: String,

fn default_listen_address() -> String {
    "127.0.0.1".to_string()
}
```

Type is `String` rather than `IpAddr` because TOML deserializes it naturally and the UI binds to a text entry. Parsing is done at validation time and at config-generation time (the generator may emit it as-is once validated).

### Validation

`AppSettings::validate_listen_address(&str) -> Result<(), ValidationError>`:
- Parse with `str::parse::<std::net::IpAddr>()`.
- Reject hostnames and empty strings.
- Both `127.0.0.1` and `0.0.0.0` are accepted; non-loopback values trigger a UI warning, not a validation error.

Validation runs in:
- The settings UI on save (errors surfaced as a toast).
- `ConfigWriter` defensively before writing — if invalid, fall back to `127.0.0.1` and log a warning so the backend never starts with a malformed listen.

### Generator changes

Both `build_inbounds` functions take the listen address from `settings.listen_address` instead of the literal `"127.0.0.1"`. No other inbound fields change.

UDP regression coverage:

- v2ray/xray: a test asserts `config["inbounds"][0]["settings"]["udp"] == true`. Loss of this flag would break UDP-over-SOCKS5 (e.g. QUIC, DNS).
- sing-box: a test asserts `config["inbounds"][0]["type"] == "mixed"` and `config["inbounds"][0].get("udp_disabled")` is `None` / not `true`. Adding a literal `"udp": true` to a sing-box mixed inbound is **wrong** — it is not a documented field for that inbound and would be ignored or rejected by stricter sing-box validators.

### Backward compatibility

- `#[serde(default = "...")]` on the new field means existing `settings.toml` files load unchanged with `127.0.0.1`. No migration script needed.
- No on-disk schema bump.
- Default behaviour is byte-identical to today for users who do not touch the new setting.

### UI

Settings page adds an entry row below the port rows: label "Listen address", placeholder `127.0.0.1`. On commit:
1. Validate the address.
2. If validation passes and the value is not loopback, show a one-shot toast: "Proxy now reachable from other hosts on this network."
3. Persist via the existing settings save path.

### Restart semantics

Listen address changes do not need new restart-required semantics: they piggy-back on the existing `socks_port` / `http_port` flow in `runtime-profiles` — changing any inbound-affecting setting while connected stages the change until the user reconnects.

## Alternatives Considered

- **Per-inbound listen address (different for SOCKS vs HTTP).** Rejected: complicates UX with no real-world demand. Anyone wanting that level of control can run two instances or post-process the generated config.
- **`Vec<IpAddr>` to listen on multiple addresses.** Rejected: v2ray/xray/sing-box inbounds accept a single `listen` string; emulating multi-bind would mean duplicating each inbound entry, which fans out routing and risks port conflicts. Not worth it for a v1.
- **Adding `udp_fragment: true` to sing-box mixed inbound to "match" v2ray.** Rejected: not a real sing-box option for the mixed inbound — would be cargo-culting. UDP on `mixed` is implicit.

## Risks

- **Security**: a user who flips listen to `0.0.0.0` without realising exposes the proxy to their LAN. Mitigated by the warning toast and by keeping the default at `127.0.0.1`.
- **Backend rejection of address**: passing an invalid address to the backend would cause it to fail to start with a confusing log. Mitigated by validation in both UI and `ConfigWriter`.
- **IPv6 literal handling**: bare `::` is valid; `::1` for loopback. No URL-encoding required because the field is consumed by the backend's own listen parser, not embedded in a URL.
