## 1. Primary vs advanced DNS layout

- [x] 1.1 Refactor `build_dns_page` so the primary section shows only the `remote` and `domestic` roles
- [x] 1.2 Move the full server list, `use_custom_rules`, custom-rule editing, FakeIP, and sing-box detour controls into Advanced
- [x] 1.3 Render "Not configured" placeholders when `remote` or `domestic` tags are missing instead of rewriting tags automatically

## 2. Validation

- [x] 2.1 Extend `DnsConfig::validate()` to reject invalid FakeIP IPv4/IPv6 CIDR ranges
- [x] 2.2 Reuse `DnsConfig::validate()` for live validation in the DNS dialogs and page-level advanced inputs
- [x] 2.3 Surface inline error styling while validation fails; suppress emit() on invalid page-level inputs; disable Apply in dialogs while validation fails

## 3. Provider presets

- [x] 3.1 Keep preset application replace-only for servers and strategy
- [x] 3.2 Reuse the existing confirmation dialog before applying a preset
- [x] 3.3 Verify preset apply leaves FakeIP, cache, client subnet, hosts, and compatible rules unchanged
