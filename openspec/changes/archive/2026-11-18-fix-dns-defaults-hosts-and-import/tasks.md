# Tasks

## 1. Default, Preset, and Legacy Migrated DNS Detours

- [x] 1.1 Set `detour: Some("proxy")` on `"remote"` and `detour: Some("direct")` on `"domestic"` in `DnsConfig::default()`, `standard_preset()`, and `migrate_legacy_server()` in `crates/core/src/models/dns.rs`, and verify model, preset, and migration unit tests pass
- [x] 1.2 Add config-generation regression tests in `crates/core/src/config/singbox.rs` and `crates/core/src/config/xray.rs` verifying default/preset `"remote"` detours via the proxy on `sing-box` and `"domestic"` routes direct on `xray` TUN while custom `detour: None` behavior stays unchanged

## 2. Host Override Strategy Consistency and Effective DNS Parity

- [x] 2.1 Update `crates/core/src/config/singbox.rs` to filter `hosts` entries under `Ipv4Only`/`Ipv6Only`, deduplicate `hosts` rule domains in first-seen order, and restrict `pinned_by_hosts` to hostnames with at least one emitted address, verifying `sing-box` unit tests cover dual-stack pins and wrong-family `Ipv4Only`/`Ipv6Only` pins
- [x] 2.2 Update `crates/core/src/config/effective_dns.rs` so `static_entries_v2ray` and single-family `static_entries_singbox` apply `matches_strategy`, and verify `EffectiveDns` unit tests match emitted `v2ray`/`xray`/`sing-box` configs

## 3. Xray JSON DNS Import, Geodata Preflight, and Deterministic Preset Slugs

- [x] 3.1 Extend `parse_xray_dns_server` in `crates/subscription/src/json_import.rs` to map `https+local://`, `quic+local://`, and `tcp+local://` to `Doh`/`Doq`/`Tcp` with `detour = Some("direct")` and skip `localhost`, `fakedns`, and custom-path DoH/H3 URLs with warnings, verifying `v2ray-rs-subscription` import tests pass
- [x] 3.2 Filter out disabled subscriptions (`.filter(|s| s.enabled)`) in `missing_geodata` in `crates/ui/src/app/config_regen.rs` and verify unit tests confirm disabled `use_imported_profile` subscriptions do not block `v2ray`/`xray` connects
- [x] 3.3 Replace `DefaultHasher` with fixed 64-bit FNV-1a in `crates/core/src/persistence/presets.rs` (with name-match fallback on delete if a legacy hash filename exists) and verify unit tests assert exact deterministic hex slugs for non-ASCII preset names
