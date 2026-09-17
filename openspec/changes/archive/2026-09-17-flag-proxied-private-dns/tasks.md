## 1. Model

- [x] 1.1 Private-server predicate on `DnsServerConfig` in `crates/core/src/models/dns.rs`, taking the backend and TUN state; table test over `127.0.0.1`, `::1`, `10.1.2.3`, `172.20.0.1`, `192.168.1.1`, `169.254.1.1`, `fe80::1`, `fd00::1`, `1.1.1.1`, `dns.google` × sing-box (`proxy`/`direct`/no detour) × xray (TUN on/off, `proxy`/`direct`/no detour) × v2ray, matching the dns-configuration scenarios; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green

## 2. Generators

- [x] 2.1 Both generators in `crates/core/src/config/v2ray.rs` and `crates/core/src/config/singbox.rs` log one warning per flagged private server during generation; unit test asserts the predicate reports `udp 127.0.0.1` with detour `proxy` flagged for the generating backend and generation still succeeds (through the predicate call, not log capture); `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green

## 3. Preferences UI

- [x] 3.1 Server dialog in `crates/ui/src/preferences/dns.rs` shows the private-server warning below the downgrade warning, refreshed on address, detour, protocol and backend changes; Save stays enabled; pure helper producing the warning text unit-tested for flagged and unflagged inputs
- [x] 3.2 `render_dns_servers` and `render_primary_dns_servers` append the warning for flagged servers; helper tests for both texts
- [x] 3.3 Strategy row subtitle for xray and v2ray stating that the Prefer options query only the preferred family, none for sing-box, refreshed on backend change; helper test per backend
- [x] 3.4 `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4` green

## 4. Verification

- [x] 4.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 4.2 Live: with the current settings (`domestic` `udp 127.0.0.1` detour `proxy`, xray with TUN) the DNS page shows the warning on Domestic, connecting still works, and the app log carries one warning naming `domestic`; switching the detour to `direct` clears the warning and the regenerated xray config tags that server `dns-direct`
- [ ] 4.3 Live: switch the backend to sing-box and back on the DNS page — the strategy note appears for xray and v2ray and is absent for sing-box