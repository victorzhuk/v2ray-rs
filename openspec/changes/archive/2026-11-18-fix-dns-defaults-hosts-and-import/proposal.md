# Proposal

## Why

Default DNS settings and built-in presets leave server `detour` unset, causing `sing-box` to dial `"remote"` DoH resolvers outside the proxy while `xray` TUN routes `"domestic"` UDP resolvers through the proxy. Additional edge cases in `hosts` strategy matching, Xray JSON DNS scheme import, geodata preflight on disabled subscriptions, and unstable non-ASCII preset hashes cause broken resolution or toolchain-dependent file paths.

## What Changes

- Set explicit role detours (`detour = Some("proxy")` on `"remote"`, `detour = Some("direct")` on `"domestic"`) in `DnsConfig::default()`, `builtin_dns_presets()`, and legacy two-field DNS migration while preserving `detour = None` behavior for custom server records.
- Deduplicate domain lists in `sing-box` `hosts` DNS rules, check single-family strategy usability (`Ipv4Only`/`Ipv6Only`) before binding `sing-box` proxy outbounds to `domain_resolver: "hosts"`, and filter `EffectiveDns` static entries for `v2ray`/`xray` by `DnsConfig.strategy`.
- Parse Xray JSON `+local` DNS schemes (`https+local://`, `quic+local://`, `tcp+local://`) into `Doh`/`Doq`/`Tcp` with `detour = Some("direct")`, and skip Xray pseudo-servers (`localhost`, `fakedns`) or unsupported custom DoH/H3 paths with surfaced import warnings instead of coercing them to UDP port 53.
- Ignore disabled subscriptions (`enabled = false`) during `v2ray`/`xray` geodata preflight even when `use_imported_profile` is `true`.
- Replace `std::collections::hash_map::DefaultHasher` in custom routing preset filenames with deterministic 64-bit FNV-1a over UTF-8 bytes so non-ASCII preset paths stay stable across Rust versions.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `dns-configuration`: Set explicit `"proxy"` and `"direct"` detours on default and migrated `"remote"`/`"domestic"` servers, deduplicate `sing-box` `hosts` rule domains, and filter `v2ray`/`xray` static entries in `EffectiveDns` by query strategy.
- `dns-provider-presets`: Set `detour = Some("proxy")` on `"remote"` and `detour = Some("direct")` on `"domestic"` across all built-in DNS provider presets.
- `config-generator`: Require a strategy-compatible IP in `dns.hosts` before setting `domain_resolver: "hosts"` on `sing-box` proxy outbounds under `Ipv4Only` or `Ipv6Only`.
- `subscription-update`: Parse Xray `+local` DNS URLs with direct detour and skip `localhost`/`fakedns` pseudo-servers or unsupported DoH paths with warnings during JSON import.
- `geodata-management`: Skip disabled subscriptions when checking `v2ray`/`xray` geodata preflight requirements.
- `app-persistence`: Use a fixed, compiler-independent hash for non-ASCII custom preset filenames.

## Impact

- `crates/core`: `models/dns.rs`, `config/singbox.rs`, `config/effective_dns.rs`, `persistence/presets.rs`
- `crates/subscription`: `json_import.rs`
- `crates/ui`: `app/config_regen.rs`
- Existing persisted `settings.toml` files with explicit `servers` arrays continue loading unchanged; fresh configs, legacy migrations, and applied presets receive explicit `"proxy"`/`"direct"` detours.
