## 1. Rotating writer

- [x] 1.1 Add the rotating file writer to `crates/core` (append, byte-counted 5 MiB threshold, three rotations, 0600 file / 0700 dir); unit tests with `TempDir` for rotation order, deletion of the oldest, and modes; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [x] 1.2 `AppPaths` exposes the logs directory under `state_dir`; test for production, development and test profiles

## 2. Application logger

- [x] 2.1 `log::Log` implementation in the UI crate writing to the application file and stderr, level from `V2RAY_RS_LOG` (default `info`); unit test that a `warn!` lands in the file and a `debug!` does not by default
- [x] 2.2 Install it first in `main`; verify by launching the dev profile and finding the startup record in the dev state directory
- [x] 2.3 Audit every `log::` call site for URLs, UUIDs and passwords and replace them with names; verify with `rg -n 'log::(info|warn|error|debug)!' crates` reviewed against the rule

## 3. Backend log

- [x] 3.1 `ProcessManager::with_log_file`: reader tasks write each line with timestamp and stream; unit test with a stub backend that emits 20,000 lines while the broadcast receiver is not polled — all lines in the file
- [x] 3.2 Session record before spawn and exit record after exit (code or signal, requested or not, crash count, last line); unit tests for a requested stop and a crash
- [x] 3.3 Connection task passes the backend log path; verify a live connect writes the session record

## 4. Failure toasts

- [x] 4.1 Geodata refresh failure toasts once per streak; test the streak logic
- [x] 4.2 Subscription auto-update failure toasts once per streak, naming the subscription; test the streak logic

## 5. Verification

- [x] 5.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [x] 5.3 `CHANGELOG.md` `[Unreleased]` and `docs/ARCHITECTURE.md` (log locations, rotation, `V2RAY_RS_LOG`); verify both mention the paths
