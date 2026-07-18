## 1. Generator

- [x] 1.1 `singbox.rs`: drop `download_detour` from remote rule-set objects
- [x] 1.2 `singbox.rs`: emit `experimental.cache_file` (`enabled: true`, absolute `path` under the profile cache dir) when any remote rule-set is referenced; `store_fakeip: true` when FakeIP is enabled
- [x] 1.3 Thread the cache path from `ConfigWriter`/`AppPaths` into the sing-box generator (implemented as a writer post-process: `apply_singbox_cache_file` in `writer.rs`, keeping the generator path-free)

## 2. Tests

- [x] 2.1 Flip `test_singbox_geoip_route` (and friends) from asserting `download_detour == "direct"` to asserting the key is absent
- [x] 2.2 New test: config with a GeoIP rule contains `experimental.cache_file.enabled == true` and an absolute path
- [x] 2.3 New test: no rule-sets → no `experimental` section; FakeIP on → `store_fakeip: true`
- [x] 2.4 `singbox_check` schema test still passes against the real binary (`sing-box check`), plus a new rule-set + cache_file case
- [ ] 2.5 Follow-up rider: `singbox_check`'s rule-set case stitches the cache_file inline because `ConfigWriter::with_dir` is `cfg(test)`-gated; consider exposing it under `test-utils` for integration tests

## 3. Verification

- [x] 3.1 `cargo test --workspace` green
- [ ] 3.2 Manual: with GitHub blackholed (e.g. bogus /etc/hosts entry), sing-box connects via a reachable proxy; second start with the proxy also blackholed still passes rule-set init from cache (live run with the fixed shape passed rule-set init and populated the cache where the old config FATALed; the explicit blackhole matrix remains)
