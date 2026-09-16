## 1. Up-front resolution

- [x] 1.1 Pure `pin_hosts(candidates, subscriptions, manual_nodes, enabled_rules, settings) -> Vec<(String, u16)>` in `crates/ui/src/connection.rs`: unique hostnames of every candidate node and its via nodes, skipping IP literals and user host overrides, empty when the candidate's effective TUN is off or backend is v2ray; unit tests: TUN off → empty, duplicate hosts deduplicated, via node included, user override skipped; `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4` green
- [x] 1.2 `resolve_pins(hosts, last_good, deadline)`: concurrent `lookup_host` (16 in flight, 5 s overall), failed or timed-out hosts take `last_good` addresses; test with an injected lookup fn: one host fails with `last_good` present → fallback used, one host times out without `last_good` → absent and logged
- [x] 1.3 Candidate loop applies the resolved map to each candidate's `effective_settings.dns.hosts` instead of calling `pin_node_addresses`; remove the per-candidate lookup; test: two-candidate stub connection with TUN off → generated config has no `hosts` entries for the node hostname

## 2. Reachability filter

- [x] 2.1 `filter_reachable(addrs, port, per_connect, total) -> Vec<IpAddr>` keeps addresses whose TCP connect succeeds, returns all when none does; tests with a local `TcpListener` on one of two loopback addresses (`127.0.0.1` listening, `127.0.0.2` closed) → only the listening one kept; both closed → both kept
- [x] 2.2 Loop runs the filter per candidate before `write_config` (1.5 s per connect, 2 s total) and skips it when a parked manager has a TUN runtime; `ProcessManager` exposes whether it holds TUN routing state; unit test on the skip predicate

## 3. Visibility

- [x] 3.1 Connection task emits, once per attempt, a notice through the log stream and `AppMsg::ShowToast` when backend is xray, TUN on, `dns_hijack == Hijack` and `hosts_cover_nodes` is false; stub test: two unpinned candidates → exactly one notice line and one toast message
- [x] 3.2 The session record keeps stating capture through the existing `capture_dns=true|false` field; an unpinned xray TUN candidate test asserts `capture_dns=false` in `backend.log`; no second field is added
- [x] 3.3 Connection task sends `AppMsg::NodePins(generation, Vec<HostOverride>)` when a candidate reaches `Running`; `app.rs` stores it for the current generation and passes it as `last_good_pins` in `ConnectionRequest`; pure helper test that a stale generation is ignored

## 4. Verification

- [x] 4.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 4.2 Live, xray TUN with a hostname node: `backend.log` session record shows `capture_dns=true`; generated `xray.json` `dns.hosts` holds only addresses that accept on the node port; with TUN off the generated config has no node host entry; forcing a failover between two hostname nodes produces no `cannot pin … lookup timed out` in `v2ray-rs.log`
