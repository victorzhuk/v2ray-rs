## 1. Exit and session records

- [ ] 1.1 `StopReason` (`UserStop`, `NodeSwitch`, `ApplyRestart`, `AppQuit`, `StartFailed`) in `crates/process/src`; `shutdown_with(reason)`; launch-failure stops use `StartFailed`; exit record prints `reason=` (`crash` on unexpected exit after readiness); a backend that exits before ready records `requested=false reason=start-failed crashes_in_window=0` from the readiness path, never through `graceful_stop`; stub-backend tests: requested stop with each reason, TUN device timeout → `start-failed`, crash → `requested=false reason=crash`, pre-ready exit → `requested=false reason=start-failed`; `timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4` green
- [ ] 1.2 `last_error_line` over the last 50 buffered lines (`[Warning]`, `[Error]`, `WARN`, `ERROR`, `FATAL`, `panic`, ANSI-tolerant) → `last_error=` in the exit record; test: warning then access lines → `last_error` is the warning, `last_output` the access line; crash `Error` text unchanged
- [ ] 1.3 `with_session_fields(String)` appended to the session line; connection task passes `hijack capture_dns strict nodes_pinned profile`; test asserts the session line for an xray TUN stub

## 2. Connection decisions

- [ ] 2.1 `ConnectionCmd::Stop(StopReason)`; `Disconnect` maps `reconnect_pending` → `ApplyRestart`, `pending_direct_target` → `NodeSwitch`, `quit` → `AppQuit`, otherwise `UserStop`; pure mapping function unit-tested in `crates/ui/src/app.rs`
- [ ] 2.2 App-log lines: `connect`, `candidate start`, `candidate failed`, `auto-reconnect scheduled`/`exhausted`, `quit requested`; test with a capturing logger or stub backend: three candidates, first fails → connect, failed 1/3 with reason, start 2/3 in order
- [ ] 2.3 `StateManager::transition` logs `backend state from → to`; `handle_unexpected_exit` logs the crash-respawn line with crash count; `timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4` green

## 3. Unclean exits

- [ ] 3.1 `glib::unix_signal_add_local` for SIGTERM/SIGINT/SIGHUP → quit path; second signal while `pending_exit` exits; verified live: `kill -TERM` while connected → app log records the signal, `backend.log` has `reason=app-quit`, TUN device gone
- [ ] 3.2 Panic hook after `init_logging`, chaining the previous hook; unit test installs the hook with a test logger and asserts message + location
- [ ] 3.3 Startup PID-record detection appends the unclean-previous-run record to `backend.log`; test with a stale PID file in a temp profile

## 4. Status bar

- [ ] 4.1 Pure `status_texts(&StatusView) -> (String, String)` over the sprint's shared `StatusView { state, prev_state, meta, origin, attempt }` → "Restarting after crash" / "Reconnecting (n/3)" / "Connecting…", with the Running/Stopping/Stopped/Error arms byte-identical to today; unit tests for each; wired into `update_status_labels`. `detect-dead-proxy-link` adds `health` and `dns_failing` to the same struct rather than a second status function

## 5. Verification

- [ ] 5.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 5.2 Live: connect, apply a pending change with restart, switch node, disconnect, quit → `backend.log` exits carry `apply-restart`, `node-switch`, `user-stop`, `app-quit`; `v2ray-rs.log` shows connect origins and candidate lines
