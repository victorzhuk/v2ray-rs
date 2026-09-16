## 1. Warning text

- [x] 1.1 Pure `v2ray_tun_warning(settings) -> Option<String>` in `crates/ui/src/connection.rs`: `Some` only for backend v2ray with `tun.enabled`, text names SOCKS/HTTP `listen_address:port` (IPv6 listen address bracketed); unit tests for v2ray+TUN (text contains `127.0.0.1:2080` and `127.0.0.1:2081`), v2ray without TUN, sing-box/xray with TUN, `::1` listen address

## 2. Surfacing

- [x] 2.1 `start_connection` in `crates/ui/src/app.rs` shows the warning via `show_toast` and still spawns the connection; verified live (toast appears, status reaches Connected)
- [x] 2.2 Connection task emits the warning once per connection as a log line through the log stream (logs page + `backend.log`), before the first candidate starts; stub-backend test: warning line present once for v2ray+TUN, absent for v2ray without TUN

## 3. Verification

- [x] 3.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 3.2 Live: TUN on, switch backend to v2ray, Connect → toast shown, logs page and `backend.log` contain the warning, `session … tun=off`
