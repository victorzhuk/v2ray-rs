## 1. WebSocket host

- [x] 1.1 `build_ws_settings` merges `ws.host` as `Host` when no header key equals `host` ignoring case; tests: v2ray headers `{"User-Agent": "x"}` + host → both present; lowercase `host` header + node host → header value kept, no second entry
- [x] 1.2 xray test: same node → `wsSettings.host` is the node host, `headers` keeps `User-Agent` without `Host`
- [x] 1.3 sing-box `build_ws_transport` uses the same case-insensitive check; test with lowercase `host` header → single host entry
- [x] 1.4 Extend `pinned_node_and_ws_transport_options_pass_xray_test` in `crates/core/tests/xray_check.rs` with headers + host; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green

## 2. v2ray refusals

- [x] 2.1 `ConfigError` variant for an unsupported security feature with a message naming node, feature, and backend; unit test on the Display text
- [x] 2.2 `V2rayGenerator::generate` refuses REALITY (new variant) and XHTTP (`UnsupportedTransport { backend: V2ray }`) before building; tests mirror `test_singbox_xhttp_unsupported_transport` for both; xray tests with REALITY/XHTTP nodes still generate
- [x] 2.3 `V2rayProbeGenerator` skips refused nodes using the same predicate (`v2ray_supports`, added here so it is never dead code); test: batch [REALITY, TLS] → one outbound tagged with index 1
- [x] 2.4 `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green

## 3. Node editor labels

- [x] 3.1 `crates/ui/src/nodes.rs`: `grpc_multi_mode` gets subtitle "Not used by sing-box"; `spider_x` title "Reality Spider X (not used by sing-box)"; values still round-trip through `TransportRows::value` / `TlsRows::value` (existing editor tests or a new one asserting `multi_mode` and `spider_x` survive); `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4` green

## 4. Verification

- [x] 4.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [x] 4.2 Live on xray: a WS node with a custom header and separate host connects, and the generated `xray.json` shows `wsSettings.host`; on v2ray, direct-connecting a REALITY node shows the error toast naming the node and REALITY, and auto-connect moves to the next candidate
