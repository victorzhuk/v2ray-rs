# Tasks: add-tun-exclude-routing

## 1. Model

- [x] 1.1 Add `exclude_processes: Vec<String>` and `exclude_domains: Vec<String>` to `TunConfig` in `crates/core/src/models/tun.rs` with `#[serde(default)]`; mirror both in `TunConfigWire` and map them in `From<TunConfigWire>`.
- [x] 1.2 Initialize both to `Vec::new()` in `Default for TunConfig`.
- [x] 1.3 Extend `TunConfig::validate()` to loop `validate_domain_pattern` over `exclude_domains` and reject `exclude_processes` entries that are empty or contain a path separator.
- [x] 1.4 Tests: defaults are empty; a legacy `[tun]` section without the new fields loads them empty; round-trip with non-empty lists; validation rejects bad domains and process names.

## 2. sing-box generator

- [x] 2.1 Give `build_route` access to `settings.tun`. When TUN is enabled, prepend `{ process_name: exclude_processes, outbound: "direct" }` (when non-empty) and `{ domain_suffix: exclude_domains, outbound: "direct" }` (when non-empty) ahead of the user rules.
- [x] 2.2 In `build_dns`, when TUN is enabled, prepend matching DNS rules so excluded processes/domains resolve via a direct (non-detour) server.
- [x] 2.3 Tests: process-name rule prepended; domain rule prepended; DNS rule emitted; nothing emitted when TUN is disabled or the lists are empty.

## 3. xray / v2ray generator

- [x] 3.1 Give `build_routing` access to `settings.tun`. When TUN is enabled, prepend `{ type: "field", ip: exclude_routes, outboundTag: "direct" }` and `{ type: "field", domain: exclude_domains, outboundTag: "direct" }` ahead of the user rules.
- [x] 3.2 In `build_dns_for_backend`, bind `exclude_domains` to the direct/domestic DNS server's `domains` list when TUN is enabled.
- [x] 3.3 Tests: ip and domain field rules prepended (bypass via the direct outbound); excluded domains present in the direct server's `domains`; nothing emitted when TUN is disabled.

## 4. UI

- [x] 4.1 In `crates/ui/src/preferences/tun.rs`, remove the sing-box-only gating from the excluded-routes group so it applies to xray too.
- [x] 4.2 Add an *Excluded domains* group (both backends) mirroring the excluded-routes list pattern, validating with `validate_domain_pattern`.
- [x] 4.3 Add an *Excluded applications* group (sing-box only) for process basenames, with an insensitive note for xray explaining it cannot match by process name.
- [x] 4.4 Update the backend-gating blocks (the static block and the `subscribe_settings` observer) so each group is gated independently.

## 5. Verification & docs

- [x] 5.1 `cargo test --workspace` green.
- [ ] 5.2 Manual: with xray TUN on, add Cloudflare's published CIDRs to Excluded routes and confirm `cloudflared tunnel` connects; with sing-box TUN on, add `cloudflared` to Excluded applications and confirm an already-running instance bypasses.
- [x] 5.3 Update `CLAUDE.md` (TunConfig `exclude_processes`/`exclude_domains`) and `CHANGELOG.md` (Added entry).
