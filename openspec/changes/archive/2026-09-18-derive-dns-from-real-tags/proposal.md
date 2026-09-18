## Why

Auto-derived DNS rules (`use_custom_rules = false`) only work when servers are tagged exactly `remote` and `domestic`: xray assigns derived domains to a server only under those tags (`crates/core/src/config/v2ray.rs`) and drops the rest without a word, while sing-box emits `"server": "remote"` / `"server": "domestic"` unconditionally (`crates/core/src/config/singbox.rs`), naming a server that is not in the config — `DnsConfig::validate` checks `rules[].server_tag` for custom rules only, so nothing catches it. The sing-box DNS plane derived for TUN (`derived_tun_dns`) emits `strategy`, `servers`, `rules` and `final` only: the enabled path sets per-server `client_subnet` and top-level `disable_cache`, and the config-generator spec already requires both on the derived path, so the derived plane silently drops what the page says it will apply.

## What Changes

- With auto-derived rules active, each generator emits derived rules only for tags that exist in the generated config, logs a warning naming the missing tag and the number of skipped domain entries, and the primary DNS rows state that derived rules for that server are skipped.
- The sing-box DNS plane derived under TUN carries `disable_cache` and the derived server's `client_subnet`, matching the user-configured path.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `dns-configuration`: new requirement for auto-derived DNS rules requiring their server tags.
- `config-generator`: "TUN mode DNS resolution is self-contained" gains a scenario for cache control and client subnet on the sing-box derived path.
- `dns-preferences-ui`: new requirement for the missing auto-split tag note.

## Impact

- `crates/core/src/config/v2ray.rs` — derived-domain tag check and warning in `build_user_dns_servers`.
- `crates/core/src/config/singbox.rs` — derived-rule tag check in `build_dns`, cache control and client subnet in `derived_tun_dns`.
- `crates/ui/src/preferences/dns.rs` — skipped-derived-rules note on the primary rows.
- `crates/core/tests/singbox_check.rs` — case for servers tagged `remote` + `lan`.