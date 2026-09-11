## 1. Direct outbound strategy

- [x] 1.1 Remove `settings.domainStrategy` from `harden_tun_outbounds`; update `test_xray_tun_freedom_uses_builtin_resolver` to assert `streamSettings.sockopt.domainStrategy` (`UseIPv4` for IPv4 strategies, `UseIPv6` for IPv6 ones) and the absence of `settings.domainStrategy`; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [x] 1.2 Test that with TUN disabled the freedom outbound carries neither field

## 2. TLS verification per backend

- [x] 2.1 Make the TLS stream builder backend-aware: emit `allowInsecure` only for v2ray; tests for xray-verify-on (field absent) and v2ray-verify-off (`true`)
- [x] 2.2 xray generation returns an error naming the node when `verify == false`; test the error text and that `connection.rs` records it as that candidate's failure and moves on (existing per-candidate failure path)

## 3. Deprecation guard

- [x] 3.1 `crates/core/tests/xray_check.rs`: fail when `xray run -test` output contains `deprecated` (case-insensitive), except the Shadowsocks and WebSocket-transport protocol notices; add a TUN + DNS-off + hostname REALITY case using a non-colliding interface name
- [x] 3.2 Run `timeout 5m cargo test -p v2ray-rs-core --test xray_check -- --test-threads=1` against the installed xray 26.9.9 and record the result in the change — Xray 26.9.9: 4 passed, 0 skipped (`generated_xray_configs_pass_xray_test`, `new_rule_kinds_pass_xray_test`, `pinned_node_and_ws_transport_options_pass_xray_test`, `tun_without_dns_and_hostname_reality_node_pass_xray_test`); no `freedom.domainStrategy` notice

## 4. Docs

- [x] 4.1 `CHANGELOG.md` `[Unreleased]`: xray nodes with verification off are skipped with an error; direct dials under TUN follow the DNS strategy; verify the entries exist
