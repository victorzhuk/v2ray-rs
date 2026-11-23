# Tasks: fix-process-session-and-privilege-safety

## 1. Compile, warning, and formatting blockers

- [x] 1.1 Replace `advisory_tx: mpsc::UnboundedSender<ProcessAdvisory>` in `crates/process/src/manager.rs:493-495` with `Arc<Mutex<Vec<mpsc::UnboundedSender<ProcessAdvisory>>>>` fan-out so `subscribe_advisories(&self)` compiles and delivers advisories losslessly.
- [x] 1.2 Fix compiler warnings: remove unused `Signal` import (`crates/process/src/pid.rs:10`), simplify irrefutable `let..else` (`crates/process/src/manager.rs:3098`), and clean unused `degraded` binding (`crates/process/src/session/mod.rs:3275`).
- [x] 1.3 Run `cargo fmt` across the workspace so `make fmt` exits 0.

## 2. Session failover, orphan cleanup, and Prepared TUN journal teardown

- [x] 2.1 Fix `crates/process/src/session/mod.rs:788-798` and `987-997`: release/disarm any *previous* `parked` manager before setting `parked = Some(mgr)`, and do not immediately `parked.take()` the manager just parked.
- [x] 2.2 Add a shared exit helper in `crates/process/src/session/mod.rs` that drains `orphaned` into `pending` before checking `pending.is_empty()`, and wire it into lines `383-388`, `438-443`, `472-477`, `505-515` (`prepare_tun_session` failure), `678-683`, `712-717`, `738-743`, `923-928`, and `972-976`.
- [x] 2.3 Move `build_antidpi_runtime` and `writer.write_config` before `prepare_tun_session` (`session/mod.rs:505-576`), and ensure `ProcessManager::stop` (`manager.rs:827-830`) invokes `teardown_tun()` when `self.tun` holds a `Phase::Prepared` journal.
- [x] 2.4 Add unit/lifecycle tests proving: (a) candidate 1 remains parked while candidate 2 starts (`probe_allowed == false`), (b) `orphaned` triggers `retry_cleanup` on `Stop` even when the active manager stops cleanly, (c) `prepare_tun_session` error tears down `parked`/`orphaned`, and (d) cancelling during preflight clears a `Phase::Prepared` journal.

## 3. Non-blocking stop, atomic PidFile, and async blocking offload

- [x] 3.1 Switch `SessionHandle::cmd_tx` (`crates/process/src/session/mod.rs:144-160, 200`) to `mpsc::UnboundedSender` so `stop()` never calls `blocking_send`.
- [x] 3.2 Update `PidFile::atomic_write_0600` (`crates/process/src/pid.rs:237-262`) to use `tempfile::NamedTempFile::new_in(parent)` with mode `0600` and add a concurrent-write test.
- [x] 3.3 Offload `User::from_name`, `file_caps_supported`, `helper_path`/`antidpi_path`, `pid_file.write`, and `tun_session` `fsync` writes in `session/mod.rs`, `manager.rs`, and `tun.rs` via `tokio::task::spawn_blocking`.

## 4. Privilege preflight and `v2ray-rs-run` hardening

- [x] 4.1 Set `.stderr(Stdio::null())` in `getfacl_access_text` (`crates/process/src/privilege.rs:1328-1358`).
- [x] 4.2 Update `mount_for_path` (`crates/process/src/privilege.rs:2269-2279`) to `continue` on blank/malformed lines and add a unit test with leading blank lines.
- [x] 4.3 Canonicalize `self.binary_path` before `file_caps_supported` and add `nosuid` preflight checks for canonical `route_helper` and `antidpi.helper_path` (`crates/process/src/manager.rs:579-765`, `crates/process/src/tun.rs:447-456`).
- [x] 4.4 Switch `crates/run/src/main.rs:44` (`sanitize_env`) from `std::env::vars()` to `std::env::vars_os()` and add a unit test with a non-UTF-8 env var.

## 5. Verification

- [x] 5.1 Run `make fmt`, `make check-all`, and `make clippy` and verify zero errors and zero warnings.
- [x] 5.2 Run `timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-run -- --test-threads=4` and verify all existing and new tests pass.
