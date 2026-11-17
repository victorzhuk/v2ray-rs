# Proposal

## Why

Runtime supervision, TUN bootstrap/failover, single-instance locking, probe sessions, persistence, tray notifications, and GTK preference/editor dialogs contain lifecycle races and edge-case bugs that can strand hidden windows without a tray, leak plaintext DNS across every subscription hostname before TUN connects, orphan `setcap` backend processes on `fs.suid_dumpable=2` kernels, misroute Real Delay probes through an active TUN interface, or discard user input on validation errors.

## What Changes

- Activate and focus the running primary window over session D-Bus when a secondary default-path launch contends on the profile lock (falling back to holder PID + exit `75` when `--*-dir` overrides are active or D-Bus activation is unavailable), avoid `O_TRUNC` / `instance.json` clobbering on lock contention, and check `IsStatusNotifierHostRegistered` before hiding the window on close.
- Resolve TUN candidate hostnames lazily per candidate when no fail-closed routes sit parked, cap upfront pre-pinning to `<= 8` planned candidates before strict `xray` TUN installs routes, reuse `last_good_pins` under parked kill-switch routes, make `os_lookup` and `probe_pins` cancellable on `Stop`, tear down pending TUN sessions before non-TUN or reconfigured connects, drop parked failed `ProcessManager`s once failover reaches `Running`, and serialize connect/disconnect commands across `Stopping`.
- Handle `/proc/<pid>/exe` `EACCES` on `setcap` binaries by verifying `(pid, start_time_ticks, cmdline, comm)` with atomic `0600` PID writes, set `stdin(Stdio::null())` and `kill_on_drop(true)` on backend spawns, drain `stdout`/`stderr` concurrently in backend version probes, track `imported_profile` / `use_imported_profile` / `config_output_dir` in `RuntimeSnapshot`, stop `chmod 0700` on custom `config_output_dir`s, use `paths.geodata_index_dir()`, add non-ASCII hash fallback in `presets::slugify`, and reject `socks_port == 0`, `http_port == 0`, or `socks_port == http_port`.
- Block `Test Real Delay` and `Check Availability` while TUN is active, hold `REAL_DELAY_LOCK` across `ProbeRunner` teardown, batch probe outbounds (`<= 64`) with scaled timeouts and bounded response reads, reuse a single gRPC channel with `ready()` and early completion in `observatory.rs`, bound `ping_nodes` lookup concurrency, and run 10 MiB subscription decode/parse work on `spawn_blocking`.
- Fix GTK4/libadwaita bugs across `dns.rs`, `network.rs`, `tun.rs`, `subscriptions.rs`, `routing.rs`, `nodes.rs`, `wizard.rs`, `tray`, `main.rs` (`LC_MESSAGES` pre-thread init, drop runtime `setlocale`), and `CONTRIBUTING.md`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `process-lifecycle`: Focus primary window via D-Bus on default-path lock contention, verify `setcap` orphan PIDs safely, track imported profile and output dir in `RuntimeSnapshot`, and guard `Stopping` transitions.
- `tun-mode`: Bound TUN hostname pre-pinning (`<= 8` lookahead for strict xray, lazy otherwise), use `last_good_pins` under parked kill-switch routes, cancel pin probes on Stop, tear down pending TUN before non-TUN connects, and drop parked failover managers on `Running`.
- `system-tray`: Verify `IsStatusNotifierHostRegistered` before minimize-to-tray, resync state on broadcast `Lagged`, escape notification markup, and clean up tray icon temp dirs and cache updates.
- `real-delay-latency-test`: Gate Real Delay and availability probes while TUN is active, hold session lock across `ProbeRunner` stop, batch loopback inbounds (`<= 64`), and cap Clash API body reads.
- `observatory-real-delay`: Reuse single gRPC channel, await `ready()`, and finish polling as soon as probed outbounds settle.
- `geodata-management`: Store `geodata_index.json` under `cache_dir/geodata-index/`.
- `backend-detection`: Drain `stdout`/`stderr` concurrently in version probes and skip `detect_all()` when onboarding is already complete.
- `app-persistence`: Add non-ASCII preset slug hash fallback, validate distinct non-zero proxy ports, flush debounced settings on Preferences close/quit, serialize subscription saves, and offload 10 MiB decode/parse to `spawn_blocking`.
- `dns-preferences-ui`: Preserve `port = None` in DNS server dialog, sync default port on protocol switch, delete exact `(domain, ip)` host overrides, and refresh Effective DNS on routing changes.
- `network-preferences-ui`: Enable apply button on listen address row, enforce `socks_port != http_port`, initialize Real Delay URL preset and clear error styling, and initialize `LC_MESSAGES` pre-thread without runtime `setlocale`.
- `tun-preferences-ui`: Guard programmatic rollback re-entrancy, parse shell argv and surface exit errors in `Run with bypass`, and run `getpwnam` off the GTK thread.
- `ui-lists`: Keep editor `AlertDialog`s open on validation errors, update subscription node rows via `NodeId` widget map instead of `AdwExpanderRow` tree walks, and preserve loaded settings and `v2raytun://import/` handling in `OnboardingWizard`.

## Impact

Affects `v2ray-rs-core`, `v2ray-rs-process`, `v2ray-rs-subscription`, `v2ray-rs-tray`, `v2ray-rs-ui`, and `CONTRIBUTING.md`. Uses existing `gtk4`/`gio` D-Bus APIs, `glib::shell_parse_argv`, and `tokio` primitives without adding new crate dependencies.
