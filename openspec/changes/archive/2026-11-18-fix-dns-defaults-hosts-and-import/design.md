# Design

## Context

See `proposal.md` for motivation. Split-horizon DNS, static host overrides, Xray JSON bundle import, geodata connect preflight, and custom preset filenames all cross the boundary between persisted models (`crates/core/src/models/`), backend config emission (`crates/core/src/config/`), `v2ray-rs-subscription`, and UI preflight (`crates/ui/src/app/config_regen.rs`).

## Goals / Non-Goals

**Goals:**
- Give default, preset, and migrated legacy `"remote"` / `"domestic"` DNS servers explicit role detours (`"proxy"` / `"direct"`) while preserving `detour: None` semantics for custom user-authored servers.
- Keep `sing-box` `hosts` rules, `pinned_by_hosts`, and `EffectiveDns` consistent with the configured `DnsStrategy`.
- Import Xray JSON `https+local://`, `quic+local://`, and `tcp+local://` DNS servers faithfully and skip `localhost` / `fakedns` / custom-path DoH entries with warnings instead of coercing them to UDP port 53.
- Ignore disabled subscriptions in `missing_geodata` preflight.
- Replace `DefaultHasher` in `preset_slug` with fixed 64-bit FNV-1a.

**Non-Goals:**
- Changing how `sing-box` or `xray` interpret `detour: None` on custom user-defined servers (`sing-box` `None` -> direct, `xray` `None` -> default route; existing `dns-configuration` spec scenarios remain intact).
- Adding arbitrary custom HTTP path fields to `DnsServerConfig` for non-`/dns-query` DoH servers.

## Decisions

### 1. Explicit `detour` on standard `"remote"` and `"domestic"` constructors only
- **Choice:** Set `detour: Some("proxy".into())` on `"remote"` and `detour: Some("direct".into())` on `"domestic"` in `DnsConfig::default()`, `standard_preset()` (used by all 8 `builtin_dns_presets()`), and `migrate_legacy_server()`. Leave `DnsServerConfig` deserialization (`#[serde(default)] pub detour: Option<String>`) and generator lowering unchanged.
- **Why over changing generator lowering of `detour: None`:** `dns-configuration/spec.md` explicitly specifies that a `sing-box` server with `detour: None` has path `direct` (`Scenario: sing-box server without detour`) and an `xray` server with `detour: None` follows default routing. Setting explicit detours on the built-in/migrated `"remote"` and `"domestic"` role servers aligns both `sing-box` and `xray` TUN split-horizon behavior without breaking `detour: None` contracts.

### 2. Strategy-aware `hosts` filtering on `sing-box` and `EffectiveDns`
- **Choice:**
  - In `singbox.rs`, filter `settings.dns.hosts` through `host.matches_strategy(settings.dns.strategy)` when strategy is `Ipv4Only` or `Ipv6Only` (retaining both families under `PreferIpv4`/`PreferIpv6`), deduplicate `hosts_domains` while preserving first-seen order, and build `pinned_by_hosts` from the emitted `hosts_map` keys so `domain_resolver: "hosts"` is only attached when at least one usable IP was emitted for that hostname.
  - In `effective_dns.rs`, filter `static_entries_v2ray` with `host.matches_strategy(settings.dns.strategy)` (matching `v2ray.rs`) and apply single-family filtering to `static_entries_singbox` when `Ipv4Only` or `Ipv6Only` is active.
- **Why:** Prevents `sing-box` from binding `domain_resolver: "hosts"` to an empty/wrong-family pin under `Ipv4Only`/`Ipv6Only`, eliminates duplicate domain strings in `dns.rules`, and keeps `EffectiveDns` identical to the emitted config.

### 3. Xray JSON DNS scheme parser with explicit `+local` and pseudo-server handling
- **Choice:** In `crates/subscription/src/json_import.rs`, recognize `https+local://`, `quic+local://`, and `tcp+local://` before URL parsing, mapping them to `(DnsProtocol::Doh | Doq | Tcp, Some("direct"))`. Reject `"localhost"` and `"fakedns"` pseudo-servers and DoH/H3 URLs whose path is neither `""`, `"/"`, nor `"/dns-query"` with warnings instead of falling through to `(DnsProtocol::Udp, raw, None)`.
- **Why:** Xray's `+local` transport suffix is standard in shared Xray JSON profiles for direct domestic resolvers; coercing unrecognized URLs or pseudo-servers into UDP `:53` produces invalid server addresses or unintended unencrypted queries.

### 4. Filter `subscriptions.iter().filter(|s| s.enabled)` in `missing_geodata` and FNV-1a in `preset_slug`
- **Choice:** Add `.filter(|s| s.enabled)` before `.filter(|s| s.use_imported_profile)` in `crates/ui/src/app/config_regen.rs`. Replace `DefaultHasher` in `crates/core/src/persistence/presets.rs` with a 10-line `fn fnv1a_64(bytes: &[u8]) -> u64` (`offset_basis = 0xcbf29ce484222325`, `prime = 0x100000001b3`).
- **Why:** Disabled subscriptions never supply candidate nodes or imported profiles at connect time; FNV-1a is zero-dependency and bit-identical across all Rust releases and targets.

## Risks / Trade-offs

- **[Risk] Existing unit tests asserting `detour == None` on `DnsConfig::default()` or `builtin_dns_presets()`** -> Mitigation: Update affected assertions and add end-to-end generator tests proving `sing-box` default `"remote"` emits `"detour": <proxy_tag>` and `xray` TUN default `"domestic"` emits `dns-direct-user` routing.
- **[Risk] Pre-existing custom presets with non-ASCII names saved under an older Rust build's `DefaultHasher` filename** -> Mitigation: `list_custom_presets` loads all `.json` files in `presets/` regardless of filename, and `delete_custom_preset` can fall back to scanning `presets/*.json` by `preset.name` if the FNV-1a path does not exist.
