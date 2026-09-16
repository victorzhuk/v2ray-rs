## 1. Readiness wait in the process manager

- [x] 1.1 Pure `AppSettings::local_endpoint(port) -> SocketAddr` (unspecified v4/v6 → loopback of that family) in `crates/core/src/models/settings.rs`, beside `validate_listen_address`, so the health probe of `detect-dead-proxy-link` calls the same helper for its own port; unit tests for `127.0.0.1`, `0.0.0.0`, `::`, `192.168.1.10`; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [x] 1.2 `ProcessManager::with_ready_probe(SocketAddr)` (re-exported from `crates/process/src/lib.rs` so `crates/ui` can call it) plus a test-settable ready timeout field (like `restart_delay`); `wait_ready` polls every 100 ms for a TCP accept and the child still alive `STABILITY_WINDOW` (1 s) after spawn, returning `ExitedBeforeReady(reason)` on exit (reason built like `handle_unexpected_exit`'s `process exited with code N: <last line>`) or a timeout error naming address and duration; unit tests with a test-owned `TcpListener` and stub scripts: `exec sleep 30` + listener → ready; `exit 1` after `echo FATAL` → `ExitedBeforeReady` containing `FATAL`; `exec sleep 30` with no listener and 300 ms timeout → timeout error and child reaped
- [x] 1.3 `start_with_connection` calls `wait_ready` after `launch()` (after `xray-up` for xray TUN) and transitions to `Running` only on success; on failure stops the child, transitions `Starting → Error`, records no crash; test: FATAL stub → `Err(ExitedBeforeReady)`, exactly one ` session ` record in `backend.log`, `crashes_in_window=0` in the exit record, no respawn after 3 s
- [x] 1.4 `respawn()` waits for readiness too; a readiness failure returns `Err` so the existing loop records a crash and retries; test: stub that serves once, crashes, then exits immediately on every respawn → `Error("3 crashes within 60s: …")` after two respawns
- [x] 1.5 A manager without `with_ready_probe` keeps spawn-as-ready behavior; existing `manager.rs` tests unchanged and green

## 2. Connection task and app

- [x] 2.1 `crates/ui/src/connection.rs` sets `with_ready_probe(effective_settings.local_endpoint(effective_settings.socks_port))` per candidate; stub-backend tests bind an ephemeral listener and set `socks_port` to it; `last_candidate_failure_reports_one_error` expects the startup-failure reason instead of `3 crashes`; `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4` green
- [x] 2.2 New stub test: two candidates, first exits with `FATAL` right after start, second serves → only one ` session ` record for the first candidate, then `Running` for the second, no `Running` reported for the first
- [x] 2.3 `app.rs` `ProcessStateConnection` persists `last_success` only when `state` is `Running` with metadata; pure helper `records_last_success(&ProcessState, Option<&ConnectionMetadata>) -> bool` unit-tested for `Starting`+meta (false), `Running`+meta (true), `Running`+none (false)

## 3. Verification

- [x] 3.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 3.2 Live: sing-box TUN on an IPv6-less host (before `fix-singbox-tun-without-ipv6` lands) → status never shows Connected, `backend.log` has one ` session ` per candidate with no 2 s/4 s respawns, `settings.toml` `last_success` unchanged; xray TUN on a working node → Connected after routes are up and `last_success` updated
