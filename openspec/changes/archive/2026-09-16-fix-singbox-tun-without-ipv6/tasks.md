## 1. Host IPv6 probe

- [x] 1.1 Add `host_has_ipv6()` (existence of `/proc/sys/net/ipv6`) to `crates/process/src/tun.rs` and export it; `timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4` green

## 2. sing-box strict route

- [x] 2.1 Pure `singbox_strict_route_allowed(backend, host_has_ipv6) -> bool` in `crates/ui/src/connection.rs`; unit tests: sing-box + no IPv6 → false, sing-box + IPv6 → true, xray + no IPv6 → true (netctl handles xray)
- [x] 2.2 Connection task clears `effective_settings.tun.strict_route` when not allowed, before `write_config`, and emits the notice line once per connection through the log stream (logs page + `backend.log`); test with a sing-box stub: generated config has `"strict_route": false`, notice appears once across two failed candidates, `settings.tun.strict_route` unchanged

## 3. IPv6 address preflight

- [x] 3.1 `start_connection` in `crates/ui/src/app.rs` rejects TUN with `address_v6` set on a host without IPv6 (sing-box and xray) via toast + `Err` before `connection::spawn`; pure predicate unit-tested for backend × v6-address × host-IPv6 combinations

## 4. Verification

- [x] 4.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 4.2 Live on a host booted with `ipv6.disable=1`: sing-box TUN connects (session `tun=on`, no FATAL in `backend.log`), notice line present, traffic flows through `tun-in`
