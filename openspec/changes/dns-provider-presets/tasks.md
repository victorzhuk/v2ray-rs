# Tasks: dns-provider-presets

## 1. DNS provider preset model

- [ ] 1.1 Add `DnsProviderPreset` struct in `crates/core/src/models/dns.rs`: name, description, servers (Vec<DnsServerConfig>), strategy (DnsStrategy)
- [ ] 1.2 Implement `builtin_dns_presets()` returning 8 presets: Cloudflare, Cloudflare Family, Google, AdGuard, AdGuard Family, Quad9, Ali DNS, Yandex DNS
- [ ] 1.3 Add `apply_dns_preset()` method on `DnsConfig`: replaces servers + strategy, enables DNS, preserves rules/fakeip/cache/hosts
- [ ] 1.4 Write unit tests: preset count, apply replaces servers, apply preserves other settings, apply enables DNS, each preset has valid server configs with unique tags

## 2. Provider picker UI

- [ ] 2.1 Add "Providers" button to DNS Servers preference group header in `build_dns_page()`
- [ ] 2.2 Implement `show_dns_providers_dialog()` using `adw::AlertDialog` with scrolled content listing providers as ActionRows (name + description + Apply button)
- [ ] 2.3 Wire Apply button: show confirmation dialog, then call `apply_dns_preset()` on settings state, emit callback, re-render server list and strategy dropdown
- [ ] 2.4 Verify provider dialog follows same pattern as `show_routing_presets_dialog()`
