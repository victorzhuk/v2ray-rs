# Tasks

## 1. Core Locks, Runtime Snapshot, Geodata Index, and Persistence (`v2ray-rs-core`)

- [x] 1.1 Fix `InstanceLock::acquire` in `crates/core/src/instance.rs` to open `v2ray-rs.lock` without `O_TRUNC` and read `instance.json` on contention without overwriting it, verifying multi-process/multi-descriptor lock contention tests preserve holder PID
- [x] 1.2 Extend `RuntimeSnapshot` in `crates/core/src/runtime_snapshot.rs` with active subscription `imported_profile` / `use_imported_profile` and `backend.config_output_dir`, stop `chmod 0700` on external `config_output_dir` in `crates/core/src/config/writer.rs`, and reject `socks_port == 0`, `http_port == 0`, or `socks_port == http_port` in `AppSettings` validation, verifying unit tests cover dirty detection and port validation
- [x] 1.3 Switch `GeodataIndexManager` to `paths.geodata_index_dir()` (`cache_dir/geodata-index/`), add non-ASCII hash fallback in `presets::slugify`, and drain `stdout`/`stderr` concurrently in `crates/core/src/backend.rs` `run_with_probe_timeout`, verifying unit tests cover Cyrillic preset names, cache-dir index paths, and >64 KiB probe output

## 2. Process Supervision & Orphan PID Verification (`v2ray-rs-process`)

- [x] 2.1 Write `backend.pid` atomically (`0600`) with `start_time_ticks` and add `/proc/<pid>/{stat,comm,cmdline}` fallback when `/proc/<pid>/exe` returns `EACCES` in `crates/process/src/pid.rs`, plus set `.stdin(Stdio::null())` and `.kill_on_drop(true)` in `crates/process/src/manager.rs`, verifying unit tests simulate `EACCES` and PID reuse

## 3. Probe Batching, Observatory Early Exit, and Non-Blocking Decode (`v2ray-rs-subscription`)

- [x] 3.1 Hold `REAL_DELAY_LOCK` across `ProbeRunner::stop`, chunk Real Delay and Availability runs into `<= 64`-node batches with scaled timeouts, cap Clash API body reads, and use `saturating_add` on timeouts in `crates/subscription/src/real_delay.rs` and `availability.rs`, verifying batching and lock-serialization tests pass
- [x] 3.2 Reuse a single `Channel`, call `grpc.ready().await`, and exit polling once all probed tags report statuses in `crates/subscription/src/observatory.rs`, plus bound `ping_nodes` concurrency and run 10 MiB subscription decode/parse on `spawn_blocking` in `fetch.rs`, verifying unit tests cover dead-node early completion

## 4. TUN Hostname Pinning, Session Teardown, D-Bus Activation, and Tray (`v2ray-rs-tray` & `v2ray-rs-ui`)

- [x] 4.1 Implement lazy per-candidate TUN hostname resolution, `<= 8` candidate lookahead before strict `xray` TUN, `last_good_pins` fast path under parked kill-switch routes, `tokio::select!` cancellation on `os_lookup`/`probe_pins`, parked-TUN teardown before non-TUN/reconfigured connects, parked-manager drop on `Running`, and `Stopping` command serialization in `crates/ui/src/connection.rs` and `crates/ui/src/app.rs`, verifying unit tests cover each state path
- [x] 4.2 Wire primary/remote D-Bus window activation on default profile paths (keeping `--*-dir` exit-`75` fallback) and `StatusNotifierWatcher.IsStatusNotifierHostRegistered` check before `minimize_to_tray` hide in `crates/ui/src/main.rs` and `crates/ui/src/app.rs`, verifying CLI override fallback tests pass
- [x] 4.3 Resync `TrayService` state on broadcast `Lagged`, abort listener task on `shutdown()`, honor `enabled` + escape markup in `Notifier`, retain/clean `TempDir` for tray icons, and run `gtk-update-icon-cache` asynchronously off the GTK thread in `crates/tray/src/`, verifying tray and notification unit tests pass

## 5. GTK4 Dialog Validation, Preferences Fixes, Wizard, and Locale Init (`v2ray-rs-ui`)

- [x] 5.1 Gate `Test Real Delay` and `Check Availability` while TUN is active, invalidate `testing_availability` in `SyncSettings`, serialize `SubscriptionPersister` with reloads, and replace `AdwExpanderRow` child-tree walking with an explicit `NodeId` widget map in `crates/ui/src/subscriptions.rs`, verifying UI state tests pass
- [x] 5.2 Keep `routing.rs`, `nodes.rs`, and `subscriptions.rs` editor dialogs open with reactive validation on invalid input instead of closing and discarding fields, verifying validator helpers cover invalid inputs
- [x] 5.3 Fix `port = None` clamping, protocol default-port sync, exact `(domain, ip)` `HostOverride` deletion, and routing-triggered `Effective DNS` refresh in `crates/ui/src/preferences/dns.rs`; enable `show_apply_button(true)` on `listen_address_row`, enforce `socks_port != http_port`, and sync `real_delay_preset_row` in `network.rs`; add rollback re-entrancy guard, off-thread `User::from_name`, `glib::shell_parse_argv`, and exit-error reporting in `tun.rs`; and flush `settings_debounce` on Preferences close and `quit()`, verifying preference unit tests pass
- [x] 5.4 Skip `detect_all()` when `onboarding_complete == true`, initialize `OnboardingWizard` from loaded `AppSettings`, strip `v2raytun://import/`, sync `NodesPage` on wizard completion, initialize `LC_MESSAGES` pre-thread in `main.rs` while dropping runtime `setlocale`, and update `CONTRIBUTING.md` (`8` crates, `~/.local/state/` logs), verifying build and startup smoke tests pass

## 6. Cross-Crate Integration Verification

- [x] 6.1 Run `cargo test --workspace` with bounded timeouts and `openspec validate fix-runtime-and-ui-lifecycle --strict` to verify all runtime, TUN, tray, and UI lifecycle contracts pass

## Workflow follow-up

- Update `CHANGELOG.md` `[Unreleased]` for D-Bus single-instance focus, TUN hostname pinning, and UI/probe fixes.
- Archive the change once implementation and verification complete.
