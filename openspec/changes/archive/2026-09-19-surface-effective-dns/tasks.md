## 1. Effective DNS model

- [x] 1.1 Add `crates/core/src/config/effective_dns.rs` with `effective_dns(backend, settings, rules, node_hosts) -> EffectiveDns` (entries `{address, transport, path, source, scope}`, `uses_fallback`); unit tests for every dns-configuration scenario; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [x] 1.2 Cross-check test: for the matrix backend × TUN × DNS enabled × scoped/unscoped servers × IP/hostname node, the set of summary addresses equals the addresses in the generated `dns.servers` (and `system` iff no `dns` section or a `localhost`/`local` server); same command green
- [x] 1.3 `source = profile` marking via a caller flag set when `resolve_effective_config` applied a profile DNS; unit test with `ImportedProfile { dns: Some(..) }`

## 2. Backend log

- [x] 2.1 `ProcessManager::with_session_extra(Vec<String>)` writes each as a `dns` stream line right after the `session` line in `launch`; stub-backend test: two `dns` lines follow `session` on start and again after a crash respawn; `timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4` green
- [x] 2.2 Connection task computes the summary from `effective_settings` (after pinning) per candidate and passes the lines; emits the fallback notice once per connection through the log stream; test with a stub backend: notice once across two failed candidates, absent with DNS enabled

## 3. Preferences

- [x] 3.1 "Effective DNS" group in `crates/ui/src/preferences/dns.rs`, sensitive regardless of the master toggle, rebuilt through the settings observers on DNS/TUN/backend change, lists imported-profile subscriptions; verified live: xray + TUN + DNS off shows both fallback DoH entries, toggling DNS on replaces them

## 4. Verification

- [x] 4.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 4.2 Live xray TUN with DNS off: `backend.log` shows `dns … source=fallback` after `session`, the logs page shows the notice once, the DNS page summary matches the generated `xray.json` `dns.servers`
