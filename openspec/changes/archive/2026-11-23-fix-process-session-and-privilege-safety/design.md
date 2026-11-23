# Design: fix-process-session-and-privilege-safety

## Context

`crates/process` owns the headless session supervisor (`session/mod.rs`), process manager (`manager.rs`), privilege grant/probes (`privilege.rs`), helper resolution (`tun.rs`), and PID file ownership (`pid.rs`). `crates/run` is the setuid-root bypass launcher. Uncommitted anti-DPI lifecycle edits left a compile error and critical teardown regressions on candidate failover and cleanup paths.

## Decisions

### 1. Lossless `ProcessManager::subscribe_advisories` (`manager.rs`)
- Replace `advisory_tx: mpsc::UnboundedSender<ProcessAdvisory>` with `advisory_subs: Arc<Mutex<Vec<mpsc::UnboundedSender<ProcessAdvisory>>>>`.
- `subscribe_advisories(&self)` creates a fresh `(tx, rx) = mpsc::unbounded_channel()`, pushes `tx` into `advisory_subs`, and returns `rx`.
- Emitting an advisory locks `advisory_subs`, sends a clone to each subscriber, and retains only open senders (`tx.send(msg.clone()).is_ok()`). Every subscriber gets lossless, ordered delivery even if the log broadcast lags.

### 2. Correct `parked` failover and unified `orphaned` cleanup (`session/mod.rs`)
- On candidate start failure (`788-798`) and crash give-up (`987-997`):
  1. First, if `mgr.teardown_antidpi().await.is_err()`: collect `mgr` into `pending`, also run `stop_or_park` on `parked.take()` into `pending`, drain `orphaned` into `pending`, and enter `retry_cleanup`.
  2. Otherwise (`mgr`'s anti-DPI teardown succeeded): inspect any *previous* `stale = parked.take()`. If `stale.tun_identity() == mgr.tun_identity()`, disarm `stale.abandon_superseded_tun()` (and drop `stale` after stopping any leftover child); else run `stop_or_park(&mut stale)` and retain `stale` in `orphaned` if cleanup fails. Only then set `parked = Some(mgr)`.
- Add one helper `finish_or_retry_cleanup(pending, &mut orphaned, ...)` that always runs `pending.extend(orphaned.drain(..))` *before* checking `pending.is_empty()`, used by every `Stop`, host-error, `prepare_tun_session` failure (`505-515`), and loop-exit branch (including `972-976`).
- Replaced-journal sessions own nothing. Draining `orphaned` into retries makes a manager whose session another session replaced in the single-slot journal unreleasable (`complete_tun_session` and `netctl xray-down` both refuse a changed session id). `tun_session_replaced` in core marks such a TUN obligation void in `ProcessManager::teardown_tun`. `tempfile` also moves from a dev-dependency to a dependency of `v2ray-rs-process` for the PID temp file.

### 3. `Prepared` TUN journal cleanup and preflight ordering (`session/mod.rs`, `manager.rs`)
- Run `build_antidpi_runtime` and `writer.write_config` before `prepare_tun_session` so pure config/validation errors never write a `Phase::Prepared` journal.
- In `ProcessManager::stop` (`manager.rs:827-830`), when `self.state.state()` is `ProcessState::Stopped` and `self.child.is_none()`, also invoke `self.teardown_tun().await` if `self.tun` carries a prepared journal (`crates/netctl/src/ownership.rs:356-370` already handles `Phase::Prepared`).

### 4. Non-blocking `SessionHandle::stop`, atomic `PidFile`, and `spawn_blocking`
- Change `SessionHandle::cmd_tx` from `mpsc::Sender` (capacity 4 + `blocking_send`) to `mpsc::UnboundedSender` so `stop()` never blocks GTK or panics on Tokio workers.
- Replace `.{file_name}.{pid}.tmp` in `PidFile::atomic_write_0600` (`pid.rs:237-262`) with `tempfile::NamedTempFile::new_in(parent)` (mode `0600`, `sync_all`, `persist`), matching `crates/core/src/fs.rs`.
- Offload `User::from_name`, `file_caps_supported`, `helper_path`/`antidpi_path`, `pid_file.write`, and `tun_session` `fsync` writes to `tokio::task::spawn_blocking`.

### 5. Privilege probes, `nosuid` preflight, and `v2ray-rs-run` env safety
- Set `.stderr(Stdio::null())` in `getfacl_access_text` (`privilege.rs:1334`) so `getfacl` stderr output can never fill the pipe buffer and deadlock.
- In `mount_for_path` (`privilege.rs:2269-2279`), use `let (Some(_), Some(mp), Some(_), Some(opts)) = ... else { continue; };` so blank or truncated lines do not abort parsing.
- In `manager.rs:579-765`, canonicalize `self.binary_path`, `route_helper`, and `antidpi.helper_path` before checking `file_caps_supported`, surfacing `nosuid`/relocation guidance before spawning.
- In `crates/run/src/main.rs:44`, iterate `std::env::vars_os()`, matching allowlisted keys via `key.to_str()` and passing through allowed `OsString` values (avoiding UTF-8 panics on dropped variables).

## Risks / Trade-offs

- **Unbounded `cmd_tx`**: Only user/UI actions emit `SessionCmd::Stop`; an unbounded channel cannot grow meaningfully while eliminating both UI deadlock and Tokio `blocking_send` panics.
- **Canonicalizing helper paths in preflight**: Symlinks resolving onto a `nosuid` mount will now fail fast at preflight with actionable relocation guidance instead of failing obscurely with `EPERM` after spawn.
