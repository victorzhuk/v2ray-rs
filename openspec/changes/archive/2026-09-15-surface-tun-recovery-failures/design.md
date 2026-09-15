## Context

- `crates/ui/src/app.rs` `fn recover_tun_session(paths: &AppPaths, helper: &std::path::Path)` runs `netctl recover` via `run_with_timeout(cmd, RECOVER_TIMEOUT)` inside `spawn_blocking`. On failure it only calls `log::warn!`, then clears the marker. `run_with_timeout` inherits stdio, so helper output goes to the app's stderr, not to `backend.log`, even though diagnostic-logs requires every route-helper line there. `RECOVER_TIMEOUT` is 5 s. Every other helper call uses `HELPER_TIMEOUT`, which is 10 s (`pub(crate) const HELPER_TIMEOUT` in `crates/process/src/tun.rs`).
- Clearing the marker on failure is deliberate and tested (`recover_clears_marker_when_helper_fails`). With a broken helper, every Disconnect, Quit and launch would otherwise rerun a failing recovery that can take up to 10 s, and it would run under the lifecycle lock. The earlier lifecycle change explicitly avoided that.
- `crates/process/src/tun.rs` `run_helper` already captures stdout and stderr lines. It kills a hung helper at its timeout, retries on ETXTBSY and keeps the lines printed before a timeout. It is `pub(crate)` and async. Helper runs that already log to `backend.log` use the `helper` stream (`write_stream_line(&self.log_writer, "helper", …)`). `RotatingFileWriter::append_line` writes `<rfc3339> <stream> <content>`.
- Recovery has two callers, both inside `tokio::spawn` and holding the TUN lifecycle lock:
  - app init, after orphan cleanup;
  - `release_tun_session`, which emits `AppMsg::TunReleased`.
- `TunSession` is `{ backend, iface }`.

## Goals / Non-Goals

**Goals:**
- A failed recovery is diagnosable: the helper output and outcome are in `backend.log`.
- A failed recovery is actionable: a toast names the manual command with the recorded backend flag and interface.
- One timeout constant covers every route-helper invocation.

**Non-Goals:**
- Keeping the TUN marker after a failed recovery (see Decisions).
- Retrying recovery automatically.
- Capability gates and version/geodata preflight (`gate-tun-capability-preflight`, `check-backend-versions-geodata`).

## Decisions

- **Surface and log the failure, and keep clearing the marker.** The rejected alternative was to keep the marker on failure. With a broken helper, every Disconnect, Quit and launch would rerun a failing recovery of up to 10 s under the lifecycle lock.
- **Recovery reuses `run_helper`.** The process crate exports `HELPER_TIMEOUT`, `HelperRun` and `run_helper`, and `HelperRun` gains `pub timed_out: bool`. `recover_tun_session` becomes `async fn … -> Result<(), RecoveryFailure>` and is awaited by both callers. Output capture, kill-on-timeout and the ETXTBSY retry then match every other helper call, and `run_with_timeout`/`RECOVER_TIMEOUT` are deleted. Orphan cleanup stays in `spawn_blocking` at startup, and recovery still runs after it under the lock.
- **Log records.** Recovery opens `RotatingFileWriter` at `logs_dir()/backend.log`; if the file cannot be opened, it logs a warning and continues. It writes each helper line as a `helper` record, then one outcome record:
  - `recover ok`
  - `recover exited with <status>` (for example, `recover exited with exit status: 1`)
  - `recover timed out after 10s`
  - `recover: <io error>`
- **`RecoveryFailure { session, timed_out }`.** The failure carries the session loaded before the marker is cleared, so callers never reload it. `recovery_hint(&TunSession)` returns `v2ray-rs-netctl recover <--singbox|--xray> --iface <iface>`. `RecoveryFailure::toast()` returns `TUN route recovery failed: run <hint>`, or `TUN route recovery timed out: run <hint>` on timeout. That satisfies the spec's "same notification stating that recovery timed out".
- **Callers.** Both emit `AppMsg::ShowToast(failure.toast())`. The release path sends it before `AppMsg::TunReleased`, which is always sent.

## Risks / Trade-offs

- [Two `backend.log` writers: recovery at startup and a connection] → both run under the TUN lifecycle lock, never concurrently. Recovery drops its writer before releasing the lock.
- [A failed recovery leaves stale routes once the marker is gone] → the toast names the manual command, and `backend.log` keeps the helper's output for diagnosis.
- [Quit path: `TunReleased` destroys the window when `pending_exit` is set] → the toast may never show, but `backend.log` still holds the outcome. Accepted.
- [The outcome text depends on the std `ExitStatus` Display (`exit status: 1`)] → stable on Linux; the tests assert it.
- [Marker load/clear and log open now run on a tokio worker] → small synchronous file calls, negligible.

## Implementation plan

Tier **standard**, mode **existing-service-strict**, lenses **spec + quality**. Plan review by zarchitect:
- Round 1: one blocker. An async `recover_tun_session` with callers still synchronous in the same chunk would fail clippy `-D warnings`.
- Round 2: **pass** after re-splitting the chunks.

Rust has no test-writer stage. Every seam carries `NO-RED-WAIVER: rust stack, tests authored by the coder as the first code task; NO-TESTER-WAIVER: verification = chunk cargo test + floor.`

Verify command for each chunk: `timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-process -p v2ray-rs-ui --all-targets --all-features -- -D warnings && cargo fmt -- --check`

### helper-api (task 1.1): first chunk, rust-coder
- Sites:
  - `pub(crate) const HELPER_TIMEOUT: Duration = Duration::from_secs(10);`, `pub(crate) async fn run_helper(`, `pub(crate) struct HelperRun` (`crates/process/src/tun.rs`)
  - `pub use tun::{` (`crates/process/src/lib.rs`)
- Change: make all three `pub` and re-export them. Add `pub timed_out: bool`, set only on timeout.
- Tests: extend `helper_killed_after_timeout` (`assert!(run.timed_out)`) and `helper_output_is_captured` (`assert!(!run.timed_out)`).

### recovery-surface (tasks 1.2, 1.3): serial after helper-api (shares the process crate's public API), rust-coder
- Sites (`crates/ui/src/app.rs`):
  - `fn recover_tun_session(`
  - `const RECOVER_TIMEOUT`
  - `fn run_with_timeout(`
  - the init comment `// process and TUN recovery up to RECOVER_TIMEOUT.`
  - the startup call `recover_tun_session(&bg_paths, &v2ray_rs_process::helper_path());`
  - `fn release_tun_session(`
  - `AppMsg::ShowToast(message) => {`
  - the recover tests
- References:
  - `RotatingFileWriter::open(paths.logs_dir().join("backend.log"), DEFAULT_MAX_BYTES)` (`connection.rs`)
  - `pub fn append_line(` (`rotating_log.rs`)
  - `pub struct TunSession {`
- Contract: the Decisions above. Both callers change in this chunk, so the async function is awaited and `RecoveryFailure` is read in the library build. No `#[allow(dead_code)]` bridges.
- Tests (`app::tests`, `#[tokio::test]`):
  - existing: `recover_runs_helper_and_clears_marker`, `recover_clears_marker_when_helper_fails` (asserts `is_err`)
  - new recovery tests: `recover_logs_helper_output_and_exit_status`, `recover_logs_ok_outcome`, `recover_without_marker_does_nothing`
  - new toast tests: `recovery_hint_names_singbox_flag_and_iface`, `recovery_hint_names_xray_flag_and_iface`, `recovery_toast_distinguishes_timeout`

### Not chunk work
- Task 2.1 (workspace test run) is covered by the floor.
- Task 2.2 (live check: helper capability removed, `kill -9` during a sing-box TUN session, relaunch) is a manual step that needs sudo and the GUI.

### Floor
`make test && make fmt && make clippy`. The Makefile applies `timeout 5m` and `--test-threads=4`.

### Rules for an executor without the orchestration tooling
- Work in a worktree off the base commit, and assert `git -C <worktree> rev-parse --show-toplevel` before any edit.
- A contract test, once written, is read-only for later chunks. Change it only through an explicit amendment with one line of why.
- Conventional Commits: subject ≤72 chars, imperative, lowercase; body lines ≤100; no `openspec/` paths in implementation commits.
- Every test run is resource-limited, as in the commands above.

## Plan appendix

```json
{
  "v": 2,
  "change": "surface-tun-recovery-failures",
  "baseSha": "5b436b53d462cbc419ecceb0884136242a6ee467",
  "generatedAt": "2026-09-15T17:53:55.320Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "estimateHours": 1,
  "chunks": [
    {
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "pkgs": [],
      "coder": "rust-coder",
      "verify": "timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-process -p v2ray-rs-ui --all-targets --all-features -- -D warnings && cargo fmt -- --check",
      "id": "helper-api",
      "taskIds": [
        "1.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": false,
      "shard": "",
      "seam": "S1-helper-api",
      "pkgDirs": [
        "crates/process/src"
      ],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/process/src/tun.rs",
          "symbol": "HELPER_TIMEOUT",
          "anchor": "pub(crate) const HELPER_TIMEOUT: Duration = Duration::from_secs(10);",
          "change": "widen to pub; also make `HelperRun` and `run_helper` pub and add `pub timed_out: bool` to HelperRun (set true only on timeout); users: xray_up, xray_down, tests"
        },
        {
          "task": "1.1",
          "file": "crates/process/src/lib.rs",
          "symbol": "pub use tun::{",
          "anchor": "pub use tun::{\n    BYPASS_USER, TunRuntime, helper_needs_relogin, helper_path, helpers_stale,",
          "change": "re-export HELPER_TIMEOUT, HelperRun, run_helper"
        }
      ],
      "contract": {
        "states": [
          "HelperRun.result",
          "HelperRun.timed_out",
          "HelperRun.output"
        ],
        "transitions": [
          {
            "input": "helper exits 0",
            "state": "result=Ok(()), timed_out=false",
            "effect": "set",
            "evidence": "crates/process/src/tun.rs run_helper `Ok(Ok(status)) if status.success() => Ok(())`"
          },
          {
            "input": "helper exits non-zero",
            "state": "result=Err(\"<verb> exited with <ExitStatus Display>\"), timed_out=false",
            "effect": "set",
            "evidence": "tun.rs `Err(format!(\"{verb} exited with {status}\"))`"
          },
          {
            "input": "helper still running at timeout",
            "state": "child killed; result=Err(\"<verb> timed out after <timeout:?>\"), timed_out=true; lines printed before expiry kept in output",
            "effect": "forced",
            "evidence": "tun.rs `Err(_) => { let _ = child.kill().await; ... }`; comment 'Readers own the buffer outside the timed wait'"
          },
          {
            "input": "spawn fails (missing/non-executable helper)",
            "state": "result=Err(\"<verb>: <io error>\"), timed_out=false, output empty",
            "effect": "set",
            "evidence": "tun.rs spawn-error early return `HelperRun { output: Vec::new(), result: Err(format!(\"{verb}: {e}\")) }`"
          },
          {
            "input": "wait() io error",
            "state": "result=Err(\"<verb>: <e>\"), timed_out=false",
            "effect": "set",
            "evidence": "tun.rs `Ok(Err(e)) => Err(format!(\"{verb}: {e}\"))`"
          }
        ],
        "forbidden": [
          "timed_out=true with result=Ok(())",
          "timed_out=true from a non-timeout arm"
        ],
        "seeding": "Only via `run_helper(stub_script, args, timeout).await` with a stub `#!/bin/sh` script (existing tests `helper_killed_after_timeout`, `helper_output_is_captured` in crates/process/src/tun.rs); never construct HelperRun by hand in a test.",
        "budgets": {
          "HELPER_TIMEOUT": "10s",
          "OUTPUT_DRAIN_TIMEOUT": "500ms (unchanged)",
          "helper_killed_after_timeout": "keeps its existing short timeout argument"
        }
      },
      "codeTasks": [
        "1.1-test: extend `helper_killed_after_timeout` (crates/process/src/tun.rs) with `assert!(run.timed_out)`; extend `helper_output_is_captured` with `assert!(!run.timed_out)`",
        "1.1-code: make HELPER_TIMEOUT/HelperRun/run_helper pub, add `timed_out` field, re-export `HELPER_TIMEOUT, HelperRun, run_helper` from crates/process/src/lib.rs; manager.rs `use crate::tun::{self, HelperRun, TunRuntime}` unchanged"
      ]
    },
    {
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "pkgs": [],
      "coder": "rust-coder",
      "verify": "timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-process -p v2ray-rs-ui --all-targets --all-features -- -D warnings && cargo fmt -- --check",
      "id": "recovery-surface",
      "taskIds": [
        "1.2",
        "1.3"
      ],
      "prev": "helper-api",
      "sharedPkg": "v2ray-rs-process public API (HelperRun, run_helper, HELPER_TIMEOUT)",
      "parallel": false,
      "shard": "",
      "seam": "S2-recover-log",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "sites": [
        {
          "task": "1.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "RECOVER_TIMEOUT",
          "anchor": "const RECOVER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);",
          "change": "delete (line 39); refs: 2624, 2629 (format), comment 839"
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "App init comment",
          "anchor": "// process and TUN recovery up to RECOVER_TIMEOUT. Recovery flushes the",
          "change": "rename to HELPER_TIMEOUT (line 839)"
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "recover_tun_session",
          "anchor": "fn recover_tun_session(paths: &AppPaths, helper: &std::path::Path) {",
          "change": "becomes `async fn recover_tun_session(paths: &AppPaths, helper: &Path) -> Result<(), RecoveryFailure>`: load marker (None → Ok), run `run_helper(helper, [recover, <flag>, --iface, <iface>], HELPER_TIMEOUT).await`, open RotatingFileWriter at paths.logs_dir().join(\"backend.log\") (open failure → log::warn and continue), append_line(\"helper\", line) per output line then one outcome (`recover ok` / `recover exited with <status>` / `recover timed out after 10s` / `recover: <io error>`), clear marker unconditionally, return Err(RecoveryFailure { session, timed_out }) on failure"
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "run_with_timeout",
          "anchor": "fn run_with_timeout(\n    mut cmd: std::process::Command,",
          "change": "delete (only caller was recover_tun_session)"
        },
        {
          "task": "1.2",
          "file": "crates/process/src/tun.rs",
          "symbol": "run_helper / HelperRun",
          "anchor": "pub(crate) async fn run_helper(helper: &Path, args: &[String], timeout: Duration) -> HelperRun {",
          "change": "reference: now pub and re-exported from v2ray_rs_process (chunk helper-api)"
        },
        {
          "task": "1.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "log_helper / write_stream_line",
          "anchor": "write_stream_line(&self.log_writer, \"helper\", &content);",
          "change": "reference pattern: output lines then `{verb} failed: {e}` on error, each append_line(\"helper\", ..); no success record written for xray-up/down (873-888, 973-977)"
        },
        {
          "task": "1.2",
          "file": "crates/core/src/rotating_log.rs",
          "symbol": "RotatingFileWriter::open / append_line",
          "anchor": "pub fn append_line(&self, stream: &str, content: &str) {",
          "change": "reference: open(path: impl Into<PathBuf>, max_bytes: u64) -> io::Result<Self> creates parent 0o700, file 0o600, appends; line format `<rfc3339> <stream> <content>\\n` (no brackets, no colon despite doc comment saying [<stream>]); DEFAULT_MAX_BYTES 5 MiB"
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "backend_log writer",
          "anchor": "match RotatingFileWriter::open(paths.logs_dir().join(\"backend.log\"), DEFAULT_MAX_BYTES)",
          "change": "reference pattern: open failure -> log::warn!(\"open backend log: {err}\") + None; passed via .with_log_file(backend_log.clone()) (211); imports `v2ray_rs_core::rotating_log::{DEFAULT_MAX_BYTES, RotatingFileWriter}` (14)"
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "tests recover_*",
          "anchor": "fn recover_clears_marker_when_helper_fails() {",
          "change": "convert existing recover tests to #[tokio::test] + .await (fail test asserts is_err); new tests per codeTasks"
        },
        {
          "task": "1.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "App init startup recovery",
          "anchor": "recover_tun_session(&bg_paths, &v2ray_rs_process::helper_path());",
          "change": "capture `sender.input_sender().clone()`; inside tokio::spawn under the lifecycle lock: orphan cleanup stays in spawn_blocking (awaited), then `if let Err(failure) = recover_tun_session(&bg_paths, &helper_path()).await { s.emit(AppMsg::ShowToast(failure.toast())) }`"
        },
        {
          "task": "1.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "release_tun_session",
          "anchor": "fn release_tun_session(&mut self, sender: &ComponentSender<Self>) {",
          "change": "await recover_tun_session in the spawned task; on Err emit AppMsg::ShowToast(failure.toast()) before the always-sent AppMsg::TunReleased"
        },
        {
          "task": "1.3",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::ShowToast / TunReleased",
          "anchor": "AppMsg::ShowToast(message) => {",
          "change": "reference: variants at 131 `ShowToast(String)`, 134 `TunReleased`; ShowToast -> self.show_toast(&message) (1081); TunReleased handler clears tun_release_in_flight, destroys window if pending_exit (1362-1368) — toast before exit may never show on Quit path"
        },
        {
          "task": "1.3",
          "file": "crates/core/src/persistence/tun_session.rs",
          "symbol": "TunSession",
          "anchor": "pub struct TunSession {",
          "change": "reference: fields backend, iface; `recovery_hint(&TunSession) -> String` builds `v2ray-rs-netctl recover <--singbox|--xray> --iface <iface>`; `RecoveryFailure::toast()` → `TUN route recovery failed: run <hint>` or `TUN route recovery timed out: run <hint>`"
        }
      ],
      "contract": {
        "states": [
          "tun_session marker file (paths.tun_session_path())",
          "backend.log helper records",
          "return value Result<(), RecoveryFailure>",
          "toast emitted (AppMsg::ShowToast)",
          "TunReleased emission"
        ],
        "transitions": [
          {
            "input": "no marker",
            "state": "marker absent; no helper spawn; backend.log untouched; Ok(())",
            "effect": "no-op",
            "evidence": "app.rs recover_tun_session `let Some(session) = load_tun_session(paths) else { return; }`"
          },
          {
            "input": "marker + helper exits 0",
            "state": "helper lines then `recover ok` in backend.log; marker cleared; Ok(())",
            "effect": "clear",
            "evidence": "tasks.md 1.2; test recover_runs_helper_and_clears_marker"
          },
          {
            "input": "marker + helper prints line, exits 1",
            "state": "that line then `recover exited with exit status: 1` in backend.log; marker cleared; Err(RecoveryFailure{session, timed_out:false})",
            "effect": "clear",
            "evidence": "tasks.md 1.2 test clause; specs/diagnostic-logs 'Recovery output is kept'"
          },
          {
            "input": "marker + helper hangs past HELPER_TIMEOUT",
            "state": "pre-timeout lines then `recover timed out after 10s`; helper killed; marker cleared; Err(timed_out:true)",
            "effect": "forced",
            "evidence": "specs/process-lifecycle 'Recovery helper hangs'; run_helper timeout arm"
          },
          {
            "input": "marker + helper path missing/not executable",
            "state": "`recover: <io error>` in backend.log; marker cleared; Err(timed_out:false)",
            "effect": "clear",
            "evidence": "run_helper spawn-error return; app.rs doc comment 'marker is cleared even when the helper fails'"
          },
          {
            "input": "marker + backend.log cannot be opened",
            "state": "no file records; log::warn; helper still runs; marker cleared; outcome returned as above",
            "effect": "clear",
            "evidence": "connection.rs `Err(err) => { log::warn!(\"open backend log: {err}\"); None }`"
          },
          {
            "input": "recover Ok(()) at startup",
            "state": "no ShowToast",
            "effect": "no-op",
            "evidence": "tasks.md 1.3 'on failure'"
          },
          {
            "input": "recover Err(timed_out:false) at startup",
            "state": "ShowToast(\"TUN route recovery failed: run v2ray-rs-netctl recover --xray --iface tun9\")",
            "effect": "set",
            "evidence": "specs/process-lifecycle 'Recovery helper fails at startup'; tasks.md 1.3"
          },
          {
            "input": "recover Err(timed_out:true)",
            "state": "ShowToast(\"TUN route recovery timed out: run v2ray-rs-netctl recover <flag> --iface <iface>\")",
            "effect": "set",
            "evidence": "specs/process-lifecycle 'Recovery helper hangs'"
          },
          {
            "input": "release path, any outcome",
            "state": "ShowToast on Err first, then TunReleased always",
            "effect": "set",
            "evidence": "app.rs release_tun_session `s.emit(AppMsg::TunReleased)`"
          },
          {
            "input": "recovery_hint SingBox session iface tun0",
            "state": "\"v2ray-rs-netctl recover --singbox --iface tun0\"",
            "effect": "set",
            "evidence": "tasks.md 1.3 both backends"
          },
          {
            "input": "recovery_hint Xray session iface tun9",
            "state": "\"v2ray-rs-netctl recover --xray --iface tun9\"",
            "effect": "set",
            "evidence": "tasks.md 1.3"
          }
        ],
        "forbidden": [
          "marker still present after recover_tun_session returns when a marker was loaded",
          "helper output written to inherited stdio (Stdio::inherit) instead of backend.log",
          "Err returned with marker kept",
          "outcome record missing or written more than once per pass",
          "backend.log written when no marker was present",
          "TunReleased not emitted after a failed release recovery (would wedge tun_release_in_flight)",
          "toast on successful recovery",
          "hint naming a flag that differs from the args passed to the helper"
        ],
        "seeding": [
          "Marker: existing `paths_with_marker(&tmp)` (save_tun_session via AppPaths::for_profile_in(AppProfile::Test, tmp.path()), backend Xray, iface tun9). Helper: existing `stub_helper(&tmp, body)`. Log read: `std::fs::read_to_string(paths.logs_dir().join(\"backend.log\"))`. Tests become `#[tokio::test]` and `.await` recover_tun_session. No timeout test at ui level (would cost 10s); timeout outcome proven by S1 `helper_killed_after_timeout` plus string derived from HELPER_TIMEOUT.",
          "Pure functions only: construct `TunSession { backend, iface }` and `RecoveryFailure { session, timed_out }` literals in app.rs tests; caller wiring is not unit-tested (GTK component), covered by manual check 2.2."
        ],
        "budgets": [
          {
            "recovery wall-clock": "<= HELPER_TIMEOUT 10s + OUTPUT_DRAIN_TIMEOUT 500ms",
            "outcome records per pass": "exactly 1"
          },
          {
            "toasts per failed pass": "exactly 1"
          }
        ],
        "callers": [
          "both callers change in this chunk so the async fn is awaited and RecoveryFailure is read in the lib build: startup keeps cleanup_orphaned_backend inside spawn_blocking (awaited), then `recover_tun_session(&paths, &helper).await` while still holding the lifecycle lock, emitting AppMsg::ShowToast(failure.toast()) on Err; release awaits it the same way and emits the toast before AppMsg::TunReleased, which is always sent",
          "no #[allow(dead_code)] bridges"
        ]
      },
      "codeTasks": [
        "1.2-test: convert `recover_runs_helper_and_clears_marker` and `recover_clears_marker_when_helper_fails` to `#[tokio::test]` + `.await` (the latter additionally asserts `.is_err()`); add `recover_logs_helper_output_and_exit_status`: stub body `echo route-busy; exit 1` -> backend.log contains ` helper route-busy` and ` helper recover exited with exit status: 1`, returned Err with timed_out false, marker cleared; add `recover_logs_ok_outcome`: stub `exit 0` -> backend.log contains ` helper recover ok`; add `recover_without_marker_does_nothing`: no marker, stub writing an args file -> Ok(()), args file absent, backend.log absent",
        "1.2-code: implement async recover_tun_session, RecoveryFailure, backend_flag; delete run_with_timeout and RECOVER_TIMEOUT; imports `v2ray_rs_core::rotating_log::{DEFAULT_MAX_BYTES, RotatingFileWriter}`",
        "1.3-test: `recovery_hint_names_singbox_flag_and_iface`, `recovery_hint_names_xray_flag_and_iface`, `recovery_toast_distinguishes_timeout` (asserts both exact toast strings)",
        "1.3-code: recovery_hint, RecoveryFailure::toast, startup and release caller wiring via AppMsg::ShowToast",
        "tasks 2.1 (workspace test run) and 2.2 (live, sudo + GUI) are not chunk work: 2.1 is covered by the floor, 2.2 stays a pending manual check"
      ]
    }
  ],
  "seams": [
    {
      "id": "S1-helper-api",
      "tasks": [
        "1.1"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests authored by the coder as the first code task; NO-TESTER-WAIVER: verification = chunk cargo test + floor. Export the route-helper runner from the process crate so the ui recovery pass reuses its piped capture, pre-timeout line retention, kill on expiry and ETXTBSY retry. Decision (a): make recover async and reuse `run_helper` instead of a second sync pipe+timeout loop in ui -- both callers already sit in `tokio::spawn`, so dropping `spawn_blocking` around recovery is the smallest change that keeps lines printed before a timeout and kills on expiry; a sync `Handle::block_on` wrapper would panic in `#[tokio::test]` bodies. crates/process/src/tun.rs: `pub const HELPER_TIMEOUT: Duration = Duration::from_secs(10);` (was pub(crate)); `pub struct HelperRun { pub output: Vec<String>, pub result: Result<(), String>, pub timed_out: bool }` (was pub(crate); `timed_out` new, true only in the `Err(_)` timeout arm, false at the spawn-error return and the normal return); `pub async fn run_helper(helper: &Path, args: &[String], timeout: Duration) -> HelperRun` (was pub(crate)). crates/process/src/lib.rs `pub use tun::{...}` adds `HELPER_TIMEOUT, HelperRun, run_helper`. `timed_out` exists so ui never string-matches the error text to pick the toast wording.",
      "contract": {
        "states": [
          "HelperRun.result",
          "HelperRun.timed_out",
          "HelperRun.output"
        ],
        "transitions": [
          {
            "input": "helper exits 0",
            "state": "result=Ok(()), timed_out=false",
            "effect": "set",
            "evidence": "crates/process/src/tun.rs run_helper `Ok(Ok(status)) if status.success() => Ok(())`"
          },
          {
            "input": "helper exits non-zero",
            "state": "result=Err(\"<verb> exited with <ExitStatus Display>\"), timed_out=false",
            "effect": "set",
            "evidence": "tun.rs `Err(format!(\"{verb} exited with {status}\"))`"
          },
          {
            "input": "helper still running at timeout",
            "state": "child killed; result=Err(\"<verb> timed out after <timeout:?>\"), timed_out=true; lines printed before expiry kept in output",
            "effect": "forced",
            "evidence": "tun.rs `Err(_) => { let _ = child.kill().await; ... }`; comment 'Readers own the buffer outside the timed wait'"
          },
          {
            "input": "spawn fails (missing/non-executable helper)",
            "state": "result=Err(\"<verb>: <io error>\"), timed_out=false, output empty",
            "effect": "set",
            "evidence": "tun.rs spawn-error early return `HelperRun { output: Vec::new(), result: Err(format!(\"{verb}: {e}\")) }`"
          },
          {
            "input": "wait() io error",
            "state": "result=Err(\"<verb>: <e>\"), timed_out=false",
            "effect": "set",
            "evidence": "tun.rs `Ok(Err(e)) => Err(format!(\"{verb}: {e}\"))`"
          }
        ],
        "forbidden": [
          "timed_out=true with result=Ok(())",
          "timed_out=true from a non-timeout arm"
        ],
        "seeding": "Only via `run_helper(stub_script, args, timeout).await` with a stub `#!/bin/sh` script (existing tests `helper_killed_after_timeout`, `helper_output_is_captured` in crates/process/src/tun.rs); never construct HelperRun by hand in a test.",
        "budgets": {
          "HELPER_TIMEOUT": "10s",
          "OUTPUT_DRAIN_TIMEOUT": "500ms (unchanged)",
          "helper_killed_after_timeout": "keeps its existing short timeout argument"
        }
      },
      "codeTasks": [
        "1.1-test: extend `helper_killed_after_timeout` (crates/process/src/tun.rs) with `assert!(run.timed_out)`; extend `helper_output_is_captured` with `assert!(!run.timed_out)`",
        "1.1-code: make HELPER_TIMEOUT/HelperRun/run_helper pub, add `timed_out` field, re-export `HELPER_TIMEOUT, HelperRun, run_helper` from crates/process/src/lib.rs; manager.rs `use crate::tun::{self, HelperRun, TunRuntime}` unchanged"
      ]
    },
    {
      "id": "S2-recover-log",
      "tasks": [
        "1.2"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests authored by the coder as the first code task; NO-TESTER-WAIVER: verification = chunk cargo test + floor. crates/ui/src/app.rs: replace `fn recover_tun_session(paths: &AppPaths, helper: &std::path::Path)` with `async fn recover_tun_session(paths: &AppPaths, helper: &std::path::Path) -> Result<(), RecoveryFailure>`. Flow: `load_tun_session(paths)` -> None => `Ok(())` (no helper run, no backend.log write). Some(session) => args `[\"recover\", backend_flag(session.backend), \"--iface\", session.iface]` where `fn backend_flag(backend: BackendType) -> &'static str` returns `--singbox` for SingBox else `--xray` (existing match, extracted so `recovery_hint` shares it); `let run = v2ray_rs_process::run_helper(helper, &args, v2ray_rs_process::HELPER_TIMEOUT).await`. Decision (b): open a fresh writer exactly as the connection does -- `RotatingFileWriter::open(paths.logs_dir().join(\"backend.log\"), DEFAULT_MAX_BYTES)` (crates/ui/src/connection.rs `let backend_log =` block); open error => `log::warn!(\"open backend log: {err}\")` and continue without file records; the writer is a local dropped at function end, which is inside the caller's lifecycle lock (design risk 'recovery drops its writer before releasing the lock'). Records via `writer.append_line(\"helper\", line)` => on-disk `<rfc3339> helper <content>`: every `run.output` line in order, then exactly one outcome record: `recover ok` on Ok; on Err the run_helper error string verbatim, which yields `recover exited with exit status: 1` (std ExitStatus Display), `recover timed out after 10s` (Debug of HELPER_TIMEOUT, so '10s' comes from the constant), `recover: <io error>` on spawn failure. No `failed:` prefix (unlike ProcessManager::log_helper). Keep `log::info!(\"recovered leftover TUN state on {}\", session.iface)` on Ok and `log::warn!(\"tun {err}\")` on Err for the app log. Then `let _ = clear_tun_session(paths);` unconditionally, then return `Err(RecoveryFailure { session, timed_out: run.timed_out })` on failure. Decision (c): `struct RecoveryFailure { session: TunSession, timed_out: bool }` (private to app.rs); the session is the one loaded before the clear, so callers need no reload. Decision (f): `run_with_timeout` has no other caller (only `recover_tun_session`) -> delete it and `const RECOVER_TIMEOUT`; update the init comment mentioning RECOVER_TIMEOUT to HELPER_TIMEOUT.",
      "contract": {
        "states": [
          "tun_session marker file (paths.tun_session_path())",
          "backend.log helper records",
          "return value Result<(), RecoveryFailure>"
        ],
        "transitions": [
          {
            "input": "no marker",
            "state": "marker absent; no helper spawn; backend.log untouched; Ok(())",
            "effect": "no-op",
            "evidence": "app.rs recover_tun_session `let Some(session) = load_tun_session(paths) else { return; }`"
          },
          {
            "input": "marker + helper exits 0",
            "state": "helper lines then `recover ok` in backend.log; marker cleared; Ok(())",
            "effect": "clear",
            "evidence": "tasks.md 1.2; test recover_runs_helper_and_clears_marker"
          },
          {
            "input": "marker + helper prints line, exits 1",
            "state": "that line then `recover exited with exit status: 1` in backend.log; marker cleared; Err(RecoveryFailure{session, timed_out:false})",
            "effect": "clear",
            "evidence": "tasks.md 1.2 test clause; specs/diagnostic-logs 'Recovery output is kept'"
          },
          {
            "input": "marker + helper hangs past HELPER_TIMEOUT",
            "state": "pre-timeout lines then `recover timed out after 10s`; helper killed; marker cleared; Err(timed_out:true)",
            "effect": "forced",
            "evidence": "specs/process-lifecycle 'Recovery helper hangs'; run_helper timeout arm"
          },
          {
            "input": "marker + helper path missing/not executable",
            "state": "`recover: <io error>` in backend.log; marker cleared; Err(timed_out:false)",
            "effect": "clear",
            "evidence": "run_helper spawn-error return; app.rs doc comment 'marker is cleared even when the helper fails'"
          },
          {
            "input": "marker + backend.log cannot be opened",
            "state": "no file records; log::warn; helper still runs; marker cleared; outcome returned as above",
            "effect": "clear",
            "evidence": "connection.rs `Err(err) => { log::warn!(\"open backend log: {err}\"); None }`"
          }
        ],
        "forbidden": [
          "marker still present after recover_tun_session returns when a marker was loaded",
          "helper output written to inherited stdio (Stdio::inherit) instead of backend.log",
          "Err returned with marker kept",
          "outcome record missing or written more than once per pass",
          "backend.log written when no marker was present"
        ],
        "seeding": "Marker: existing `paths_with_marker(&tmp)` (save_tun_session via AppPaths::for_profile_in(AppProfile::Test, tmp.path()), backend Xray, iface tun9). Helper: existing `stub_helper(&tmp, body)`. Log read: `std::fs::read_to_string(paths.logs_dir().join(\"backend.log\"))`. Tests become `#[tokio::test]` and `.await` recover_tun_session. No timeout test at ui level (would cost 10s); timeout outcome proven by S1 `helper_killed_after_timeout` plus string derived from HELPER_TIMEOUT.",
        "budgets": {
          "recovery wall-clock": "<= HELPER_TIMEOUT 10s + OUTPUT_DRAIN_TIMEOUT 500ms",
          "outcome records per pass": "exactly 1"
        }
      },
      "codeTasks": [
        "1.2-test: convert `recover_runs_helper_and_clears_marker` and `recover_clears_marker_when_helper_fails` to `#[tokio::test]` + `.await` (the latter additionally asserts `.is_err()`); add `recover_logs_helper_output_and_exit_status`: stub body `echo route-busy; exit 1` -> backend.log contains ` helper route-busy` and ` helper recover exited with exit status: 1`, returned Err with timed_out false, marker cleared; add `recover_logs_ok_outcome`: stub `exit 0` -> backend.log contains ` helper recover ok`; add `recover_without_marker_does_nothing`: no marker, stub writing an args file -> Ok(()), args file absent, backend.log absent",
        "1.2-code: implement async recover_tun_session, RecoveryFailure, backend_flag; delete run_with_timeout and RECOVER_TIMEOUT; imports `v2ray_rs_core::rotating_log::{DEFAULT_MAX_BYTES, RotatingFileWriter}`"
      ]
    },
    {
      "id": "S3-recovery-toast",
      "tasks": [
        "1.3",
        "2.1",
        "2.2"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests authored by the coder as the first code task; NO-TESTER-WAIVER: verification = chunk cargo test + floor. Pure `fn recovery_hint(session: &TunSession) -> String` returns the manual command `v2ray-rs-netctl recover <backend_flag> --iface <iface>` (literal binary name; the resolved helper path may be a relocated copy, the user types the package name). `impl RecoveryFailure { fn toast(&self) -> String }` returns `TUN route recovery failed: run <hint>` when !timed_out and `TUN route recovery timed out: run <hint>` when timed_out. Decision (d): task 1.3 literal kept verbatim for exit/spawn failures; the spec's hang scenario ('same notification stating that recovery timed out') is satisfied by the same command sentence with 'timed out' in place of 'failed' -- reconcilable, not a blocker. Decision (e): init has `sender: ComponentSender<Self>` (app.rs `fn init(`); clone `let s = sender.input_sender().clone();` before the startup `tokio::spawn`, run `cleanup_orphaned_backend` alone inside `spawn_blocking(...).await`, then `if let Err(failure) = recover_tun_session(&bg_paths, &v2ray_rs_process::helper_path()).await { s.emit(AppMsg::ShowToast(failure.toast())); }` while still holding `_lifecycle`. Emitting before the component loop runs is safe: the input channel buffers. Release path (`fn release_tun_session`): replace the spawn_blocking with the same `.await`, emit `AppMsg::ShowToast(failure.toast())` before the existing `s.emit(AppMsg::TunReleased)`; TunReleased stays payload-free. If pending_exit destroys the window on TunReleased the toast is lost -- acceptable, backend.log keeps the record.",
      "contract": {
        "states": [
          "toast emitted (AppMsg::ShowToast)",
          "TunReleased emission"
        ],
        "transitions": [
          {
            "input": "recover Ok(()) at startup",
            "state": "no ShowToast",
            "effect": "no-op",
            "evidence": "tasks.md 1.3 'on failure'"
          },
          {
            "input": "recover Err(timed_out:false) at startup",
            "state": "ShowToast(\"TUN route recovery failed: run v2ray-rs-netctl recover --xray --iface tun9\")",
            "effect": "set",
            "evidence": "specs/process-lifecycle 'Recovery helper fails at startup'; tasks.md 1.3"
          },
          {
            "input": "recover Err(timed_out:true)",
            "state": "ShowToast(\"TUN route recovery timed out: run v2ray-rs-netctl recover <flag> --iface <iface>\")",
            "effect": "set",
            "evidence": "specs/process-lifecycle 'Recovery helper hangs'"
          },
          {
            "input": "release path, any outcome",
            "state": "ShowToast on Err first, then TunReleased always",
            "effect": "set",
            "evidence": "app.rs release_tun_session `s.emit(AppMsg::TunReleased)`"
          },
          {
            "input": "recovery_hint SingBox session iface tun0",
            "state": "\"v2ray-rs-netctl recover --singbox --iface tun0\"",
            "effect": "set",
            "evidence": "tasks.md 1.3 both backends"
          },
          {
            "input": "recovery_hint Xray session iface tun9",
            "state": "\"v2ray-rs-netctl recover --xray --iface tun9\"",
            "effect": "set",
            "evidence": "tasks.md 1.3"
          }
        ],
        "forbidden": [
          "TunReleased not emitted after a failed release recovery (would wedge tun_release_in_flight)",
          "toast on successful recovery",
          "hint naming a flag that differs from the args passed to the helper"
        ],
        "seeding": "Pure functions only: construct `TunSession { backend, iface }` and `RecoveryFailure { session, timed_out }` literals in app.rs tests; caller wiring is not unit-tested (GTK component), covered by manual check 2.2.",
        "budgets": {
          "toasts per failed pass": "exactly 1"
        }
      },
      "codeTasks": [
        "1.3-test: `recovery_hint_names_singbox_flag_and_iface`, `recovery_hint_names_xray_flag_and_iface`, `recovery_toast_distinguishes_timeout` (asserts both exact toast strings)",
        "1.3-code: recovery_hint, RecoveryFailure::toast, startup and release caller wiring via AppMsg::ShowToast"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "### Requirement: Backend output survives the application The system SHALL append every line the backend writes to stdout or stderr, and every line the route helper writes — including during a TUN route-recovery pass run outside a connection — to a backend log file under the state directory. Each backend launch SHALL be preceded by a session record naming the backend, its version, the node, and whether TUN is on, and each backend exit SHALL be followed by an exit record stating the exit code or signal, whether the stop was requested, the number of crashes in the current window, and the last output line. A route-recovery pass SHALL additionally record its outcome (success, exit status, or timeout).",
      "tests": [
        "v2ray-rs-ui app::tests::recover_logs_helper_output_and_exit_status",
        "v2ray-rs-ui app::tests::recover_logs_ok_outcome"
      ]
    },
    {
      "shall": "- **THEN** `<state_dir>/logs/backend.log` SHALL still contain the backend's last output lines and an exit record marking the exit as unrequested",
      "tests": [
        "existing, unchanged by this change (process manager exit record tests)"
      ]
    },
    {
      "shall": "- **THEN** every line SHALL still be written to the backend log file",
      "tests": [
        "existing, unchanged by this change (rotating_log / manager log tests)"
      ]
    },
    {
      "shall": "- **THEN** every line the route helper printed and the pass's outcome SHALL appear in `<state_dir>/logs/backend.log`, and none SHALL be written only to the application's standard error",
      "tests": [
        "v2ray-rs-ui app::tests::recover_logs_helper_output_and_exit_status",
        "v2ray-rs-ui app::tests::recover_logs_ok_outcome"
      ]
    },
    {
      "shall": "### Requirement: Route recovery failures are visible When a TUN route-recovery pass fails or times out, the system SHALL notify the user with the manual recovery command for the recorded backend and interface, and SHALL log the failure. The recovery pass SHALL be bounded by the same timeout as other route-helper invocations.",
      "tests": [
        "v2ray-rs-ui app::tests::recovery_hint_names_singbox_flag_and_iface",
        "v2ray-rs-ui app::tests::recovery_hint_names_xray_flag_and_iface",
        "v2ray-rs-ui app::tests::recovery_toast_distinguishes_timeout",
        "v2ray-rs-process tun::tests::helper_killed_after_timeout"
      ]
    },
    {
      "shall": "- **THEN** the user SHALL see a notification containing the manual recovery command naming the backend flag and the interface",
      "tests": [
        "v2ray-rs-ui app::tests::recovery_hint_names_singbox_flag_and_iface",
        "v2ray-rs-ui app::tests::recovery_hint_names_xray_flag_and_iface",
        "v2ray-rs-ui app::tests::recovery_toast_distinguishes_timeout",
        "manual: task 2.2 live check"
      ]
    },
    {
      "shall": "- **THEN** it SHALL be killed and the user SHALL see the same notification stating that recovery timed out",
      "tests": [
        "v2ray-rs-process tun::tests::helper_killed_after_timeout",
        "v2ray-rs-ui app::tests::recovery_toast_distinguishes_timeout"
      ]
    }
  ],
  "testHarness": [
    "paths_with_marker(tmp) app.rs:2463 — AppPaths::for_profile_in(AppProfile::Test, tmp.path()) + TunSession{backend: Xray, iface: \"tun9\"}; logs_dir under tmp state dir",
    "stub_helper(tmp, body) app.rs:2476 — writes #!/bin/sh script `netctl` 0o755 in tmp; ETXTBSY not handled (std Command, existing tests pass)",
    "recover_runs_helper_and_clears_marker app.rs:2485 — asserts args `recover --xray --iface tun9`",
    "recover_clears_marker_when_helper_fails app.rs:2499 — stub `exit 1`, asserts marker cleared",
    "no SingBox marker fixture exists; recovery_hint test builds TunSession directly for BackendType::SingBox and Xray",
    "run: timeout 10m cargo test -p v2ray-rs-ui -- recover --test-threads=4; workspace: timeout 10m cargo test --workspace -- --test-threads=4"
  ],
  "floor": "make test && make fmt && make clippy",
  "risks": [
    "Test module in app.rs: confirm the `mod tests` holding paths_with_marker/stub_helper is `#[cfg(test)]` inside app.rs so private RecoveryFailure/recovery_hint are reachable; tests read `paths.logs_dir()` (AppPaths::for_profile_in Test) -> mitigation: same accessor connection.rs tests use.",
    "run_helper uses tokio::spawn for readers -> `#[tokio::test]` (current_thread) suffices; switch to flavor=\"multi_thread\" only if the drain times out.",
    "Stub helper ETXTBSY: freshly written script exec can hit ETXTBSY; run_helper's spawn_with_etxtbsy_retry covers it (old sync spawn did not).",
    "Exit outcome text depends on std ExitStatus Display (`exit status: 1`); tests assert that exact form -> stable on Linux.",
    "Toast lost when Quit (pending_exit) destroys the window right after TunReleased -> backend.log retains the outcome; accepted.",
    "Startup recovery no longer inside spawn_blocking: marker load/clear and log open are small sync fs calls on a tokio worker -> negligible; orphan cleanup (~1.5s) stays in spawn_blocking."
  ],
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
