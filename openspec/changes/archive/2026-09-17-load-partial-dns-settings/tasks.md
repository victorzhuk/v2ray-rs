## 1. Model

- [x] 1.1 `DnsConfigWire` in `crates/core/src/models/dns.rs`: `#[serde(default)]` on `enabled`, `servers: Option<Vec<DnsServerConfig>>`; `From<DnsConfigWire>` takes `Some(v)` as written and migrates legacy or substitutes the defaults only on `None`; tests: `[dns]` without `enabled` loads with `enabled = false`, `servers = []` round-trips empty, a missing `servers` key with no legacy fields gets the two defaults, the existing legacy migration tests still pass; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [x] 1.2 `load_settings` test in `crates/core/src/persistence/settings.rs`: a file whose `[dns]` lacks `enabled` and whose non-DNS sections carry non-default values loads with those values preserved and no `CorruptConfig`
- [x] 1.3 `dns_server_address_for_backend` in `crates/core/src/config/v2ray.rs` formats a downgraded server with `port = None`; tests: v2ray DoT `dns.google` port 853 → `https://dns.google/dns-query`, xray H3 port 8443 → `https://dns.google/dns-query`, native DoH port 8443 keeps `:8443`; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green

## 2. Verification

- [x] 2.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 2.2 Live: remove `enabled` from `[dns]` in a copy of the settings file under a dev profile, start the app, and confirm every other setting is intact and the DNS page shows DNS disabled