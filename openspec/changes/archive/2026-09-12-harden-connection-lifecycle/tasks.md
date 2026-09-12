## 1. Route helper

- [x] 1.1 `xray-up --strict`: add the lowest-priority `unreachable` default route to table 2023 for IPv4 and IPv6, and install the IPv6 pref 9000/9001/9002 rules even without `--addr6`; verify with a `privileged-tests` netns test asserting the routes and rules exist
- [x] 1.2 `xray-down` and `recover --xray` remove the fallback routes and IPv6 rules; verify with a netns test that leaves table 2023 and prefs 8998–9002 empty for both families
- [x] 1.3 Netns test: `xray-up`, delete and recreate the device, `xray-up` again → exactly one copy of each route and rule; netns test: with strict state and no device, an unmarked connect to a public address fails with `EHOSTUNREACH` while an fwmark-255 socket still reaches `main`

## 2. Process manager

- [x] 2.1 Allow `Starting → Stopping`; `stop()` with no child drives any non-terminal state to `Stopped`; unit tests for stop from `Running`-without-child and from `Starting`
- [x] 2.2 `xray_up`/`xray_down`: 10s timeout, `kill_on_drop(true)`, piped output pushed into the log buffer; `teardown_tun` logs failures; unit test with a stub helper that sleeps past the timeout and one that writes to stderr
- [x] 2.3 `TunRuntime` carries `strict` from `tun.strict_route` and passes `--strict`; unit test on the built argument list
- [x] 2.4 `handle_unexpected_exit`: remove the pre-respawn `teardown_tun`, loop respawn failures into the crash budget, and respawn through a path that skips version, capability and config checks; unit tests with a stub backend: respawn failure retried, budget exhaustion → `Error`, no `xray-down` invocation between crash and respawn
- [x] 2.5 Give-up leaves routing state in place; teardown runs only from `stop()`; unit test asserting no helper call on give-up

## 3. Connection task and app

- [x] 3.1 Forwarder relays only `Starting`/`Running`/`Stopping`; the task emits `Stopped` after every requested stop and one `Error` after the last candidate; test the failover sequence with stub managers: app sees no `Stopped`, handle kept
- [x] 3.2 Write the TUN marker in the connection task from `TunRuntime` before `start_with_connection`; remove the write on `Running` in `app.rs`; test marker content and ordering
- [x] 3.3 Kill-switch release: when automatic reconnects are exhausted, on Disconnect with no handle, and on Quit, run the marker-driven route-recovery pass then clear the marker; verify each path in app-level tests
- [x] 3.4 Quit while `Stopping` sets `pending_exit` and waits for `Stopped`; verify the window is destroyed only after `Stopped`
- [x] 3.5 Window button and tray item enabled with "Disconnect" during `Starting`; verify with the existing button/menu state tests extended for `Starting`

## 4. Preferences and docs

- [x] 4.1 `strict_route` row note states it applies to both backends and what it blocks under xray; verify the row is sensitive for xray
- [x] 4.2 Update `docs/ARCHITECTURE.md` (crash path, kill-switch ownership, helper `--strict`) and `CHANGELOG.md` `[Unreleased]` with the IPv6 behavior change; verify both mention `strict_route`

## 5. Verification

- [x] 5.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 5.2 Privileged helper suite green: `cargo test -p v2ray-rs-netctl --features privileged-tests` under the project's netns runner
- [ ] 5.3 Live, xray TUN with strict on: `kill -SEGV` the backend while `curl` loops against a public host → requests fail with host-unreachable during the gap, none succeed via the real interface (check with `ip route get` and `ss -tnp`), tunnel returns, requests resume
- [ ] 5.4 Live: Disconnect during the respawn window → UI reaches Disconnected, `ip rule` shows no prefs 8998–9002, table 2023 empty
