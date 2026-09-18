## 1. Generators

- [x] 1.1 `build_user_dns_servers` in `crates/core/src/config/v2ray.rs`: when derived domains exist for a missing `remote`/`domestic` tag, log a warning naming the tag and the number of skipped entries; test: servers tagged `remote` + `lan` with a direct domain rule → no server carries the derived domain in `domains`, generation succeeds; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [x] 1.2 Add a `singbox_check` case (`crates/core/tests/singbox_check.rs`) with servers tagged `remote` + `lan` and a direct GeoSite rule, and record whether the current output passes `sing-box check` before the sing-box change lands
- [x] 1.3 `build_dns` in `crates/core/src/config/singbox.rs` emits derived rules only for tags present in the generated servers, with the warning from 1.1; unit test asserts no rule carries `"server": "domestic"` and the case from 1.2 passes
- [x] 1.4 `derived_tun_dns` in `crates/core/src/config/singbox.rs` adds `disable_cache` and the derived server's `client_subnet`; test mirrors `test_dns_disable_cache` / `test_dns_client_subnet` with DNS disabled and TUN on
- [x] 1.5 `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green

## 2. Preferences UI

- [x] 2.1 `render_primary_dns_servers` in `crates/ui/src/preferences/dns.rs` states that routing-derived DNS rules for that server are skipped when `use_custom_rules` is false and the `remote`/`domestic` tag has no configured server; helper test for the note text
- [x] 2.2 `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4` green

## 3. Verification

- [ ] 3.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 3.2 Live: sing-box with TUN on, DNS off, `disable_cache = true` and a client subnet set — the generated config carries both on the derived plane; with servers tagged `remote` + `lan` the app log names `domestic` once and the generated config references no `domestic` server