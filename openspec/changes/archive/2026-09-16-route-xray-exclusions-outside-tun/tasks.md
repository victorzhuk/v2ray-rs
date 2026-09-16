## 1. Route helper

- [x] 1.1 `validate.rs`: exclusion parser on top of `parse_cidr` refusing prefix 0, clearing host bits, and a count cap of 256; unit tests: `10.15.12.100/32` ok, `10.1.2.3/8` → `10.0.0.0/8`, `fd00::1/64` → `fd00::/64`, `0.0.0.0/0` and `::/0` refused, `1.2.3.4/33` refused, 257 values refused
- [x] 1.2 `main.rs`: `XrayUp` gains repeatable `--exclude`; all values validated before `net::connect()`; clap test next to `xray_up_parses_strict_flag` parses two values, invalid value exits with an error naming it
- [x] 1.3 `net.rs`: `RULE_PREF_EXCLUDE = 8997`; `replace_exclusions`, run by `xray-up` after the device setup succeeds, deletes existing pref-8997 rules for both families, then adds one destination-prefix rule to `main` per exclusion (IPv6 only when `host_has_ipv6()`), EEXIST tolerated; `is_xray_rule` includes 8997; unit test for the IPv6 skip decision as a pure function
- [ ] 1.4 Privileged namespace tests in `crates/netctl/tests/privileged.rs`: up with two exclusions → `ip rule` shows two pref-8997 rules and `ip route get` for an excluded address resolves via `main`; up with `--capture-dns` → excluded `:53` destination resolves via `main`; re-up with one exclusion → exactly one rule; `xray-down` and `recover --xray` leave no pref-8997 rule; `sudo -E timeout 10m cargo test -p v2ray-rs-netctl --features privileged-tests -- --test-threads=1` green
- [x] 1.5 `timeout 5m cargo test -p v2ray-rs-netctl -- --test-threads=4` green

## 2. Process and connection plumbing

- [x] 2.1 `TunRuntime.exclude_routes: Vec<String>`; `xray_up_args` appends `--exclude <cidr>` per entry; extend `xray_up_args_include_strict_when_set` / `xray_up_args_omit_strict_when_off` fixtures and add a test for two exclusions in order; `timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4` green
- [x] 2.2 `build_tun_runtime` copies effective `tun.exclude_routes` for xray; unit test: xray runtime carries the list, sing-box runtime has none; a runtime built from effective settings whose `exclude_routes` differ from the persisted ones carries the effective list
- [x] 2.3 `crates/core/src/models/validation.rs`: `validate_exclude_route` = `validate_ip_cidr` plus refusing prefix length 0; `TunConfig::validate` uses it for `exclude_routes` and refuses more than 256 entries; `crates/ui/src/preferences/tun.rs` excluded-route row validation uses it; unit tests: `0.0.0.0/0` and `::/0` refused, `10.0.0.0/8` ok, 257 routes refused; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green

## 3. Preferences wording

- [x] 3.1 `crates/ui/src/preferences/tun.rs`: excluded-routes description "CIDRs routed outside the tunnel; applies on next connect"; excluded-domains description per backend (xray: still enters the tunnel, sent direct by xray; sing-box unchanged) refreshed on backend change; pure helper returning the text per backend unit-tested; `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4` green

## 4. Verification

- [x] 4.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 4.2 Live on xray TUN with exclusions `10.15.12.100/32` and `91.230.107.224/32`, DNS hijack on: `ip rule` lists both at pref 8997; `ip route get 91.230.107.224` shows the physical interface; after 10 minutes of VPN use `backend.log` has no `[tun-in -> direct]` line for either address; Disconnect removes the rules
- [ ] 4.3 Live check for the open question: with TUN on, `curl --connect-timeout 2 http://192.168.10.20:1/` and a TLS client to a LAN host; if `backend.log` shows `>> proxy` for the LAN address, regenerate with `"routeOnly": true` in `tun-in` sniffing and repeat; record the outcome in this change before archiving
