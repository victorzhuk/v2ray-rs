# Proposal: fix-process-session-and-privilege-safety

## Why

Review and build checks of `crates/process` and `crates/run` uncovered a compile failure, critical failover/teardown leaks, concurrency hazards, and privilege-preflight gaps:

- `v2ray-rs-process` fails to compile (`crates/process/src/manager.rs:494`): `advisory_tx` is an `mpsc::UnboundedSender`, which has no `.subscribe()` method. Three compiler warnings (`pid.rs:10`, `manager.rs:3098`, `session/mod.rs:3275`) and `cargo fmt` drift also break `make lint` and `make check-all`.
- On candidate start failure (`session/mod.rs:788-798`) and crash give-up (`session/mod.rs:987-997`), `parked = Some(mgr)` overwrites any previously parked manager and is immediately followed by `if let Some(mut stale) = parked.take()`, dropping the just-parked manager without releasing its TUN routes. `parked` is always `None` after failover, breaking the TUN kill-switch parking contract and leaking fail-closed routes on disconnect.
- Every `Stop` and host-error branch in `session/mod.rs` (`383-388`, `438-443`, `472-477`, `678-683`, `712-717`, `738-743`, `923-928`, `972-976`) skips draining `orphaned` managers when `pending.is_empty()`, silently dropping managers with pending kernel rule/route cleanup. `prepare_tun_session` failure (`505-515`) similarly returns early without cleaning `parked` or `orphaned`.
- `prepare_tun_session` writes an Xray `Phase::Prepared` journal before `build_antidpi_runtime` and `write_config`; cancelling during preflight leaves an unretirable journal on disk that blocks switching backend/interface or next launch.
- `SessionHandle::stop` falls back to `blocking_send` on a 4-slot channel (`session/mod.rs:144-160`), risking a GTK main-thread freeze or Tokio worker panic.
- `PidFile::atomic_write_0600` (`pid.rs:237-262`) uses a PID-only `.tmp` filename and unlinks it on error, creating a same-process race. Several `async fn` paths invoke blocking NSS (`User::from_name`), `/proc/self/mounts`, and `fsync` I/O on Tokio worker threads.
- `getfacl_access_text` (`privilege.rs:1328-1358`) pipes `stderr` without draining it (deadlock risk); `mount_for_path` (`privilege.rs:2269-2279`) aborts on any blank/short line; startup preflight skips `nosuid` checks for `route_helper`/`antidpi` and fails to canonicalize `binary_path` before checking `file_caps_supported`.
- `v2ray-rs-run` (`crates/run/src/main.rs:44`) calls `std::env::vars()`, which panics on any non-UTF-8 inherited environment variable.

## What Changes

- Replace `ProcessManager`'s broken `advisory_tx.subscribe()` with a lossless multi-subscriber fan-out (`Arc<Mutex<Vec<mpsc::UnboundedSender<ProcessAdvisory>>>>`) so `subscribe_advisories(&self)` compiles and delivers every advisory without broadcast lag; fix all three compiler warnings and run `cargo fmt`.
- Fix failover parking in `session/mod.rs:788-798` and `987-997` so prior `parked` managers are properly released/disarmed before parking `mgr` and `mgr` remains parked for subsequent candidates and disconnect teardown.
- Introduce one shared exit helper in `session/mod.rs` that always drains `orphaned` into `pending` before deciding between `Stopped`/`Error` and `retry_cleanup`, covering every stop/error exit and `prepare_tun_session` failure.
- Move `build_antidpi_runtime` and `write_config` ahead of `prepare_tun_session`, and ensure `ProcessManager::stop` cleans up `Phase::Prepared` TUN journals even when `child.is_none()`.
- Replace `SessionHandle::cmd_tx` bounded channel + `blocking_send` with an unbounded channel so `stop()` is non-blocking and runtime-safe.
- Switch `PidFile::atomic_write_0600` to `tempfile::NamedTempFile::new_in` and move blocking NSS (`User::from_name`), mount checks, and `fsync` writes to `spawn_blocking`.
- Fix `getfacl_access_text` (`.stderr(Stdio::null())`), fix `mount_for_path` to skip malformed lines, canonicalize `binary_path` and add `nosuid` preflight checks for `route_helper` and `antidpi.helper_path`, and switch `v2ray-rs-run` to `std::env::vars_os()`.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `process-lifecycle`: Lossless advisory subscription compiles and delivers to subscribers; failover parking, orphan cleanup drainage, `Prepared` TUN journal teardown, non-blocking `SessionHandle::stop`, and race-free PID writes.
- `tun-mode`: Startup preflight canonicalizes backend paths and checks `nosuid` on privileged helpers; `getfacl` and `/proc/self/mounts` probes handle stderr and blank lines cleanly; `v2ray-rs-run` tolerates non-UTF-8 inherited env vars.

## Impact

- **Modified crates**: `crates/process` (`manager.rs`, `session/mod.rs`, `pid.rs`, `privilege.rs`, `tun.rs`), `crates/run` (`src/main.rs`), plus `cargo fmt` cleanup across modified files.
- **Dependencies**: None new.
