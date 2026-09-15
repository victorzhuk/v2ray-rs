## 1. Recovery surfacing

- [x] 1.1 `HELPER_TIMEOUT` exported from `crates/process/src/lib.rs`
- [x] 1.2 `recover_tun_session` pipes helper stdout/stderr, writes each line and the outcome (`recover ok`, `recover exited with <status>`, `recover timed out after 10s`) to `backend.log` as `helper` records, returns the failure, still clears the marker; `RECOVER_TIMEOUT` replaced by `HELPER_TIMEOUT`; tests: stub helper printing a line and exiting 1 → `backend.log` holds the line and the exit outcome, marker cleared, `recover_clears_marker_when_helper_fails` still green
- [x] 1.3 Startup and release callers toast `TUN route recovery failed: run v2ray-rs-netctl recover <--xray|--singbox> --iface <iface>` on failure via `AppMsg::ShowToast`; pure `recovery_hint(&TunSession) -> String` unit-tested for both backends

## 2. Verification

- [x] 2.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 2.2 Live: with the helper capability removed (`sudo setcap -r` on the helper), `kill -9` the app during a sing-box TUN session and relaunch → the recovery toast names `recover --singbox --iface <iface>` and `backend.log` holds the helper's output and exit status
