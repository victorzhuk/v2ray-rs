## 1. Runtime snapshot

- [x] 1.1 Add TUN settings and the idle-timeout / WS-heartbeat values to `RuntimeConfigSnapshot` capture, `diverges_from` and `restore_settings`; unit tests that each field raises divergence; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [x] 1.2 TUN-active state for the subscriptions page reads the launched snapshot while connected (the marker is already written from the launched settings and the tray has no TUN indicator); tests: enable TUN mid-session → probes not suppressed; a session launched without TUN writes no marker

## 2. Direct session target

- [x] 2.1 Replace `direct_connect_in_flight` with a session target kept until a user-ended session; tests for set, keep across `Running`, clear on Disconnect
- [x] 2.2 `ApplyAndRestart` and `AutoReconnect` route through the session target when set; missing/disabled target toasts and falls back to `Connect`; tests for both paths
- [x] 2.3 Existing `error_replays_pending_direct_target_*` and `stopped_replays_pending_direct_target_*` tests keep their names and replay/pending assertions without the removed in-flight argument; the two in-flight-only tests become `error_without_pending_target_replays_nothing` and `stopped_without_pending_target_replays_nothing`

## 3. Settings and timers

- [x] 3.1 `FlushSettings` keeps the app's current `last_success`; test: stale dialog copy saved after a successful connect leaves `last_success` unchanged
- [x] 3.2 A user `Connect` cancels a pending automatic reconnect (`Disconnect` and a resolvable `ConnectToNode` already do; an unresolvable direct connect is not an attempt and leaves it armed); `AutoReconnect`-originated connects neither cancel it nor reset the budget; tests for the timer generation

## 4. Log lines

- [ ] 4.1 `ProcessLogLine` carries the connection generation; mismatches dropped; test with interleaved generations

## 5. Verification

- [x] 5.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 5.2 Live: direct-connect node X, edit a routing rule, Apply & Restart → status bar shows X; toggle TUN while connected → banner appears
