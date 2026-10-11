## 1. Probe config: per-node inbounds

- [x] 1.1 Extend every `ProbeConfigGenerator` in `crates/core/src/config/probe.rs` to accept per-node loopback HTTP inbounds: N inbounds on distinct `127.0.0.1:0`-allocated ports, tag `egress-<idx>`, plus one routing rule per inbound mapping its tag to `probe_tag(idx)`; unit tests per backend for inbound count, loopback bind, tag mapping, and route rules; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [x] 1.2 Port allocation helper binds `127.0.0.1:0`, records the port, closes the socket, and rejects duplicates within one config; unit test for distinctness across N=3

## 2. Runner: egress fetch

- [x] 2.1 In `crates/subscription/src/real_delay.rs`, after each node's delay result, issue `GET` through `http://127.0.0.1:<port>` to the echo endpoint with the configured probe timeout, cap the body at 64 KiB, parse `ip=` and `loc=` keys; unit tests against a local stub HTTP server for success, timeout, non-200, and malformed body; `timeout 5m cargo test -p v2ray-rs-subscription -- --test-threads=4` green
- [x] 2.2 Egress failure never alters the node's delay result or the session exit code; unit test with a stub that never answers

## 3. Persistence

- [x] 3.1 Add optional per-node egress sample (`address`, `location`, `checked_at`) to the latency snapshot model in `crates/core/src/resolve.rs` and the node models; atomic write through the existing persist path; roundtrip unit test; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [x] 3.2 Hydrate egress samples at startup alongside delay samples; unit test for startup hydration from a fixture snapshot

## 4. UI

- [ ] 4.1 Node row subtitle in `crates/ui/src/subscriptions.rs`: `exit: <loc> · <address>` when a sample exists, nothing otherwise; formatting unit test; verified live: two same-named nodes from different subscriptions show different exits after one Real Delay run

## 5. Verification

- [x] 5.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 5.2 Live xray session: run Real Delay over nodes from two vpndio subscriptions; node rows show distinct egress locations matching `curl`-verified exits; no interference with the active TUN connection
