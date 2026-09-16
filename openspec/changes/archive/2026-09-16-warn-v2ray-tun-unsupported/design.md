## Context

- `build_tun_runtime` (`crates/ui/src/connection.rs`, anchor `fn build_tun_runtime(settings: &AppSettings, nodes_pinned: bool) -> Option<TunRuntime> {`) returns `None` for v2ray, and the v2ray generator emits no tun inbound; the session record logs `tun=off`.
- `tun_active_for` (`crates/ui/src/app.rs`) already treats v2ray sessions as non-TUN, so probes and indicators are correct. Only the user-facing explanation is missing.
- `start_connection` (`crates/ui/src/app.rs`) already surfaces preflight problems through `show_toast`; every existing call there is paired with `return Err(...)`. The connection task forwards log lines with the connection generation.
- There is a working precedent for a once-per-connection notice: the `drop_strict_route` block in `connection.rs`, anchored at `// Decided once per connection so the notice cannot repeat per candidate.`, writes `append_line("notice", ...)` on the connection's single `backend_log` writer and emits `AppMsg::ProcessLogLine(generation, ...)`, strictly before `'candidates: for candidate in candidates`.

## Goals / Non-Goals

**Goals:**
- The reason system traffic is not proxied is visible at the moment of connecting and in the persisted log.

**Non-Goals:**
- Blocking the connection or clearing `tun.enabled`: switching back to sing-box/xray should restore TUN without re-enabling it.
- Raising the v2ray log level to show per-connection access lines.

## Decisions

- **Warn, don't fail.** v2ray as a local SOCKS/HTTP proxy is a valid use; failing would break it for users who left TUN on.
- **Toast plus a log line, identical text.** A toast is transient; the log line survives in `backend.log` for later debugging. One helper builds the text, and the log line carries no added prefix, so the two are byte-identical — which is what the spec's "one line with the same warning" requires. The level is carried by `append_line`'s stream tag, not baked into the content. (The `STRICT_ROUTE_NOTICE` precedent bakes `notice: ` into its const *and* passes `"notice"` as the stream, so its on-disk line labels itself twice; not copied here.)
- **Name the real endpoints.** Built from `listen_address`, `socks_port`, `http_port` of the settings used for the connection, so the message tells the user exactly what to configure in their apps.
- **Bracket IPv6 by parsing, not by scanning for a colon.** `addr.parse::<std::net::Ipv6Addr>().is_ok()` decides; a hostname listen address is never bracketed. No bracketing helper exists anywhere in the workspace, so this is new code.
- **The toast call site ships with the helper.** `v2ray-rs-ui` has a lib target, and the chunk verify runs `cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings`, which compiles the lib without `cfg(test)`. A helper whose only caller is a test fails the chunk on `dead_code`, so task 2.1 is landed in the same chunk as task 1.1 rather than deferred.
- **Once-per-connection is structural, not a flag.** The emission is straight-line code in the connection task body, before the candidate loop, reached exactly once per spawned task; a task is spawned exactly once per Connect with a freshly bumped generation. The test proves it with two candidates — with one candidate the assertion would pass even if the emission sat inside the loop.

## Risks / Trade-offs

- [Warning on every connect gets ignored] → it only appears in the inconsistent state (v2ray + TUN flag), which the user can clear in Preferences.
- [A later refactor moves the emission inside the candidate loop, with no compile error] → the `logged-once` test runs two candidates and asserts a count of exactly 1 in both sinks, the same protection `STRICT_ROUTE_NOTICE` has.
- [The spec's `2080`/`2081` are not the defaults (`1080`/`1081`), so a test that forgets to set them drifts from the spec text] → the ports are required parameters of the `v2ray_settings` test helper, so every test states them.
- [Copying the neighbouring preflight gate's shape would add a `return Err` and block a valid v2ray SOCKS/HTTP session] → the call site is spelled as a bare `if let Some(..) { show_toast }` with no return, and task 3.2 verifies the status still reaches Connected.
- [`backend_log` is an `Option`; on an open failure only the stream line survives] → accepted; the strict-route notice has the identical exposure.

## Implementation plan

Tier **light**, testing mode **existing-service-strict**, review lens **spec**. Two chunks, serial — one crate (`v2ray-rs-ui`), one integration worktree, no shards.

Rust has no test-writer agent in this pipeline, so both seams carry `NO-RED-WAIVER:` and `NO-TESTER-WAIVER:` with that reason: the coder writes the tests first inside its own chunk, and the chunk closes by waiver rather than through a red-command run. The existing tests are still guarded — the crate's other test files are pre-sealed before the coder and guarded after, and only the test module the plan names may change.

### Chunk `warning-text` — tasks 1.1, 2.1

`prev`: none. Sites:

- `crates/ui/src/connection.rs` — `v2ray_tun_warning` (new), anchored at `fn build_tun_runtime(settings: &AppSettings, nodes_pinned: bool) -> Option<TunRuntime> {`
- `crates/ui/src/connection.rs` — `mod tests`, anchored at `    fn tun_settings() -> AppSettings {`
- `crates/ui/src/app.rs` — `App::start_connection`, anchored at `        let host_has_ipv6 = v2ray_rs_process::host_has_ipv6();`

Contract — states `warn`, `silent`, `toast-shown`, `toast-absent`:

| input | state | effect |
|---|---|---|
| backend=V2ray, `tun.enabled`=true | warn | `Some(text naming SOCKS and HTTP endpoints)` |
| backend=V2ray, `tun.enabled`=false | silent | `None` |
| backend=SingBox or Xray, `tun.enabled`=true | silent | `None` |
| `listen_address` parses as `Ipv6Addr` | warn | text contains `[::1]:<port>` |
| `listen_address` is IPv4 or a hostname | warn | text contains `<addr>:<port>`, unbracketed |
| Connect, backend=V2ray + TUN | toast-shown | set; one toast, connection still spawned |
| Connect, backend=V2ray, TUN off | toast-absent | no-op |
| IPv6 preflight gate fires | toast-absent | forced — `start_connection` returns `Err` before the site |

Forbidden: `Some` for any backend but V2ray; `Some` with `tun.enabled` false; mutating settings or clearing `tun.enabled`; returning `Err` or panicking (the helper is total over `AppSettings`); reading the running process, the host or the filesystem; a `return Err` after the toast; a second copy of the text in `app.rs`.

Code (tests first):

1. **1.1a** — add `fn v2ray_settings(socks: u16, http: u16, listen: &str, tun: bool) -> AppSettings` (TUN state is the fourth parameter, never a field mutation at the call site) and four tests: `v2ray_with_tun_warns_with_both_endpoints`, `v2ray_without_tun_does_not_warn`, `singbox_and_xray_with_tun_do_not_warn`, `ipv6_listen_address_is_bracketed`.
2. **1.1b** — implement `pub(super) fn v2ray_tun_warning(settings: &AppSettings) -> Option<String>` and the private `fn listen_endpoint(addr: &str, port: u16) -> String`.
3. **2.1** — in `start_connection`, after the `tun_ipv6_unavailable` gate block and before the routing-rules load: `if let Some(warning) = crate::connection::v2ray_tun_warning(&self.settings) { self.show_toast(&warning); }`. No `return Err`. No automated test — `show_toast` needs a live `ToastOverlay` and a GTK main loop, so it is verified live under task 3.2.

### Chunk `surfacing` — task 2.2

`prev`: `warning-text` (`sharedPkg` `crates/ui`). It depends on that chunk for both the `v2ray_tun_warning` symbol and the `v2ray_settings` test helper, and both chunks append to the same `mod tests`, so the pair must never be parallelized or reordered. Sites:

- `crates/ui/src/connection.rs` — `spawn_with`, anchored at `        // Decided once per connection so the notice cannot repeat per candidate.`
- `crates/ui/src/connection.rs` — `mod tests`, anchored at `    async fn singbox_tun_without_ipv6_turns_strict_route_off_once() {`

Contract — states `logged-once`, `not-logged`:

| input | state | effect |
|---|---|---|
| Connect, backend=V2ray + TUN | logged-once | set; exactly 1 line in the stream and 1 occurrence in `backend.log`, whatever the candidate count |
| Connect, backend=V2ray, TUN off | not-logged | no-op |
| Connect, backend=SingBox or Xray + TUN | not-logged | no-op |
| candidate 2..N starts after candidate 1 failed | logged-once | no-op — the site is outside the loop |
| a queued Stop returns the task before the loop | not-logged | no-op — the Stop check precedes the site |
| `RotatingFileWriter::open` failed, `backend_log` is `None` | logged-once | set for the stream only, forced no-op for the file |

Forbidden: emitting inside the `'candidates:` loop; opening a second `RotatingFileWriter`; a second copy of the text; mutating `settings.tun.enabled`; raising the v2ray log level; adding a `warning: ` or any prefix to the content.

Code (tests first):

1. **2.2a** — two `#[tokio::test(flavor = "multi_thread")]` tests modelled on `singbox_tun_without_ipv6_turns_strict_route_off_once`. Both use **two** candidates: with one, a `count() == 1` assertion passes even for an emission inside the loop and proves nothing. `v2ray_tun_warning_logged_once_per_connection` binds `let expected = v2ray_tun_warning(&settings).expect("warning");` and asserts exactly one matching line in the stream and one occurrence in `backend.log`. `v2ray_without_tun_logs_no_warning` derives `expected` the same way from a TUN-on settings value, then asserts its absence under TUN-off settings — so rewording the sentence cannot make the negative test pass vacuously.
2. **2.2b** — add the emission block after the `drop_strict_route` block and before `'candidates:`: `if let Some(warning) = v2ray_tun_warning(&settings) { if let Some(log) = &backend_log { log.append_line("warning", &warning); } sender.emit(AppMsg::ProcessLogLine(generation, warning)); }`.

### Commands

- Per-chunk verify: `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings`
- Floor: `timeout 10m cargo test --workspace -- --test-threads=4`

### Verification and review

Task 3.1 is the floor run. Task 3.2 is manual and is the only check on the toast: TUN on, backend switched to v2ray, Connect → toast shown, logs page and `backend.log` carry the warning, the session record still reads `tun=off`. Neither is owned by a seam.

Review lens: `spec` (light tier, no auth/migration/schema/concurrency signal in the diff).

Plan review: **pass**, reviewer zarchitect, 2 rounds. Round 1 raised two blockers — a chunk that could not pass its own `clippy -D warnings` because the helper had no non-test caller, and a seam that spelled one candidate in its seeding path and two in its code task, which would have made the once-per-connection assertion vacuous. Both are answered above.

## Plan appendix

```json
{
  "v": 2,
  "change": "warn-v2ray-tun-unsupported",
  "baseSha": "ff7e4509fd25e9b45940c35448a97e03665048a1",
  "generatedAt": "2026-09-16T06:29:28.118Z",
  "tier": "light",
  "mode": "existing-service-strict",
  "lenses": [
    "spec"
  ],
  "estimateHours": 0.5,
  "chunks": [
    {
      "id": "warning-text",
      "taskIds": [
        "1.1",
        "2.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": false,
      "seam": "warning-text",
      "shard": "",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "v2ray_tun_warning (new pure fn, sibling of build_tun_runtime / singbox_strict_route_allowed)",
          "anchor": "fn build_tun_runtime(settings: &AppSettings, nodes_pinned: bool) -> Option<TunRuntime> {",
          "change": "Add pub(super) fn v2ray_tun_warning(settings: &AppSettings) -> Option<String> next to build_tun_runtime; Some only when settings.backend.backend_type == BackendType::V2ray && settings.tun.enabled; text names settings.listen_address with settings.socks_port and settings.http_port, bracketing the address when it parses as Ipv6Addr."
        },
        {
          "task": "1.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "mod tests (unit tests beside tun_settings/strict_route_settings)",
          "anchor": "    fn tun_settings() -> AppSettings {",
          "change": "Add four #[test] cases plus the helper fn v2ray_settings(socks: u16, http: u16, listen: &str, tun: bool) -> AppSettings beside the existing tun_settings(): v2ray+TUN with ports 2080/2081 asserts the text contains 127.0.0.1:2080 and 127.0.0.1:2081; v2ray without TUN -> None; SingBox and Xray with TUN -> None; listen_address \"::1\" -> text contains [::1]:<port>."
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::start_connection",
          "anchor": "        let host_has_ipv6 = v2ray_rs_process::host_has_ipv6();",
          "change": "After the existing TUN_IPV6_DISABLED preflight (which returns Err), call crate::connection::v2ray_tun_warning(&self.settings); on Some show_toast(&msg) and fall through — no early return, the connection still spawns."
        }
      ],
      "contract": {
        "states": [
          "warn",
          "silent",
          "toast-shown",
          "toast-absent"
        ],
        "transitions": [
          {
            "input": "backend=V2ray, tun.enabled=true",
            "state": "warn",
            "effect": "Some(text naming SOCKS and HTTP endpoints)",
            "evidence": "spec scenario 'Connecting with v2ray while TUN is enabled warns' (openspec/changes/warn-v2ray-tun-unsupported/specs/tun-mode/spec.md:10-12)"
          },
          {
            "input": "backend=V2ray, tun.enabled=false",
            "state": "silent",
            "effect": "None",
            "evidence": "spec scenario 'No warning without the TUN flag' (spec.md:14-16)"
          },
          {
            "input": "backend=SingBox, tun.enabled=true",
            "state": "silent",
            "effect": "None",
            "evidence": "spec.md:14-16; TUN is real there, build_tun_runtime returns Some (connection.rs:440-469)"
          },
          {
            "input": "backend=Xray, tun.enabled=true",
            "state": "silent",
            "effect": "None",
            "evidence": "same as SingBox"
          },
          {
            "input": "backend=SingBox|Xray, tun.enabled=false",
            "state": "silent",
            "effect": "None",
            "evidence": "spec.md:14-16"
          },
          {
            "input": "listen_address parses as Ipv6Addr (e.g. `::1`)",
            "state": "warn",
            "effect": "text contains `[::1]:<socks>` and `[::1]:<http>`",
            "evidence": "task 1.1 'IPv6 listen address bracketed'"
          },
          {
            "input": "listen_address is IPv4 or a hostname",
            "state": "warn",
            "effect": "text contains `<addr>:<port>` unbracketed",
            "evidence": "task 1.1"
          },
          {
            "input": "Connect with backend=V2ray, tun.enabled=true",
            "state": "toast-shown",
            "effect": "set (one toast, connection still spawned)",
            "evidence": "app.rs tun_ipv6_unavailable gate shape one block above; spawn still reached; spec.md scenario \"Connecting with v2ray while TUN is enabled warns\""
          },
          {
            "input": "Connect with backend=V2ray, tun.enabled=false",
            "state": "toast-absent",
            "effect": "no-op",
            "evidence": "spec.md scenario \"No warning without the TUN flag\""
          },
          {
            "input": "Connect with backend=SingBox|Xray, tun.enabled=true",
            "state": "toast-absent",
            "effect": "no-op",
            "evidence": "spec.md scenario \"No warning without the TUN flag\""
          },
          {
            "input": "IPv6 preflight gate fires (tun_ipv6_unavailable)",
            "state": "toast-absent",
            "effect": "forced (start_connection returns Err before reaching the site)",
            "evidence": "app.rs: the tun_ipv6_unavailable block returns Err(TUN_IPV6_DISABLED)"
          }
        ],
        "forbidden": [
          "Returning Some for any backend other than V2ray — build_tun_runtime (connection.rs:440-447) already grants TUN to sing-box and xray; a warning there contradicts that guard.",
          "Returning Some when tun.enabled is false — the toggle state is the only trigger; nothing else in the pipeline is consulted.",
          "The helper mutating settings or clearing tun.enabled — non-goal, design.md:13.",
          "The helper returning Err or panicking — it is total over AppSettings; there is no failure path.",
          "The helper reading the running process, the host, or the filesystem — it is pure; the stub-backend test in the second seam depends on that.",
          "Showing the toast with a `return Err(..)` after it — that would block a valid v2ray SOCKS/HTTP session; design.md 'Warn, don't fail'.",
          "Writing a second copy of the text in app.rs instead of calling `v2ray_tun_warning` — one helper so toast and log stay identical.",
          "Clearing or mutating `settings.tun.enabled` at the toast site — non-goal."
        ],
        "seeding": [
          "warn: build `AppSettings { backend.backend_type: BackendType::V2ray, tun: TunConfig { enabled: true, ..default }, listen_address, socks_port, http_port }` by field assignment on `AppSettings::default()`, the same shape `tun_settings()` (connection.rs:1489-1497) and `singbox_settings()` (connection.rs:796-800) already use. The spec's 2080/2081 are NOT the defaults (defaults are 1080/1081, crates/core/src/models/settings.rs:223-224), so the test must set both ports explicitly.",
          "silent: the same construction with `tun.enabled = false`, or with `backend_type` set to `BackendType::SingBox` / `BackendType::Xray`.",
          "toast-shown / toast-absent: no automated seeding path — `show_toast` requires a live ToastOverlay and a GTK main loop; reached only by the manual run in task 3.2 (TUN on, backend switched to v2ray, Connect)."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.1a (tests, written first) — add one helper and four unit tests to the existing `mod tests` in crates/ui/src/connection.rs. The helper's exact signature is `fn v2ray_settings(socks: u16, http: u16, listen: &str, tun: bool) -> AppSettings`: `AppSettings::default()` with `backend.backend_type = BackendType::V2ray`, `tun.enabled = tun`, `listen_address = listen.to_string()`, `socks_port = socks`, `http_port = http` — the TUN state is the fourth parameter, never a field mutation at the call site. The spec's 2080/2081 are not the defaults (1080/1081), so every test states its ports. Tests: `v2ray_with_tun_warns_with_both_endpoints` — `v2ray_settings(2080, 2081, \"127.0.0.1\", true)` is Some and the text contains `127.0.0.1:2080`, contains `127.0.0.1:2081`, and contains `v2ray`. `v2ray_without_tun_does_not_warn` — `v2ray_settings(2080, 2081, \"127.0.0.1\", false)` returns None. `singbox_and_xray_with_tun_do_not_warn` — loops over [BackendType::SingBox, BackendType::Xray], overriding `backend.backend_type` on a TUN-on v2ray_settings, asserts None for each. `ipv6_listen_address_is_bracketed` — `v2ray_settings(2080, 2081, \"::1\", true)` text contains `[::1]:2080` and `[::1]:2081` and does not contain `::1:2080`.",
        "1.1b — implement `v2ray_tun_warning` and `listen_endpoint` in crates/ui/src/connection.rs exactly as spelled in this seam's summary.",
        "2.1 — add the `show_toast` call to `start_connection` in crates/ui/src/app.rs at the site spelled in this seam's summary; no automated test, verified live under task 3.2."
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings",
      "coder": "rust-coder"
    },
    {
      "id": "surfacing",
      "taskIds": [
        "2.2"
      ],
      "prev": "warning-text",
      "sharedPkg": "crates/ui",
      "parallel": false,
      "seam": "surfacing",
      "shard": "",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [
        "v2ray-rs-ui"
      ],
      "sites": [
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with — connection task, once-per-connection notice block before the 'candidates loop",
          "anchor": "        // Decided once per connection so the notice cannot repeat per candidate.",
          "change": "Mirror the drop_strict_route notice: compute v2ray_tun_warning(&settings) once here and, when Some, backend_log.append_line(\"warning\", &warning) plus sender.emit(AppMsg::ProcessLogLine(generation, warning)) — stream tag \"warning\", content the helper sentence verbatim with no added prefix; placed after the backend_log open and before `'candidates: for candidate in candidates`."
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "mod tests (stub-backend tests beside singbox_tun_without_ipv6_turns_strict_route_off_once)",
          "anchor": "    async fn singbox_tun_without_ipv6_turns_strict_route_off_once() {",
          "change": "Add two #[tokio::test(flavor = \"multi_thread\")] cases using stub()/request()/spawn_with(..., capless_probe)/drain(): v2ray+TUN settings -> exactly one matching line in `lines` and count 1 in logs_dir()/backend.log; v2ray without TUN -> line absent in both."
        }
      ],
      "contract": {
        "states": [
          "logged-once",
          "not-logged"
        ],
        "transitions": [
          {
            "input": "Connect with backend=V2ray, tun.enabled=true",
            "state": "logged-once",
            "effect": "set (exactly 1 line in the ProcessLogLine stream and exactly 1 occurrence in backend.log, regardless of candidate count)",
            "evidence": "connection.rs:142-150 precedent, assertion pattern connection.rs:1253-1257"
          },
          {
            "input": "Candidate 2..N starts after candidate 1 failed, backend=V2ray + TUN",
            "state": "logged-once",
            "effect": "no-op (no second emission; the site is outside the loop)",
            "evidence": "connection.rs:152 loop boundary"
          },
          {
            "input": "Connection task returns early on a queued Stop before the loop (connection.rs:106-109)",
            "state": "not-logged",
            "effect": "no-op (the Stop check precedes the emission site)",
            "evidence": "connection.rs:106-109"
          },
          {
            "input": "`RotatingFileWriter::open` failed, backend_log is None, backend=V2ray + TUN",
            "state": "logged-once",
            "effect": "set for the stream only, forced no-op for the file",
            "evidence": "connection.rs:131-135 makes backend_log Option and connection.rs:143 already guards with `if let Some(log)`"
          }
        ],
        "forbidden": [
          "Emitting the log line inside the `'candidates:` loop (connection.rs:152) — that yields one line per candidate and breaks the once-per-connection contract.",
          "Opening a second RotatingFileWriter for the warning — connection.rs:127-136 states one writer spans the connection.",
          "Clearing or mutating `settings.tun.enabled` at either site — non-goal, design.md:13.",
          "Raising the v2ray backend log level to surface the warning — non-goal, design.md:14.",
          "Adding a `warning: ` (or any) prefix to the emitted content — the log line must equal the helper's sentence so it matches the toast; the level is the `append_line` stream tag."
        ],
        "seeding": [
          "logged-once: `spawn_with(request(&stub, v2ray_settings(2080, 2081, \"127.0.0.1\", true), vec![candidate(\"203.0.113.1\"), candidate(\"203.0.113.2\")]), tx, capless_probe)` against `stub(r#\"exit 1\"#)` — TWO candidates, matching the precedent, because with one candidate a `count() == 1` assertion is satisfied by an emission site inside the loop too and proves nothing about once-ness. Every stub invocation fails, so `check_config` rejects each candidate and the task reaches a terminal `Error` immediately; `drain` then returns the collected lines. This is the only legal seeding path: the warning is produced by the running connection task, never by writing a line into the buffer or the file directly. v2ray never builds a TUN device (build_tun_runtime returns None), so no interface_name collision with concurrent tests.",
          "not-logged: the same stub and two candidates with `v2ray_settings(2080, 2081, \"127.0.0.1\", false)`.",
          "Never seed by constructing a ProcessLogLine message by hand or by appending to backend.log: both bypass the emission site under test."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "2.2a (tests, written first) — add two `#[tokio::test(flavor = \"multi_thread\")]` tests to the existing `mod tests` in crates/ui/src/connection.rs, modelled on `singbox_tun_without_ipv6_turns_strict_route_off_once`. `v2ray_tun_warning_logged_once_per_connection`: stub `exit 1`, settings `v2ray_settings(2080, 2081, \"127.0.0.1\", true)`, TWO candidates (`candidate(\"203.0.113.1\")`, `candidate(\"203.0.113.2\")`); `drain` gives a terminal `ProcessState::Error`; bind `let expected = v2ray_tun_warning(&settings).expect(\"warning\");` and assert `lines.iter().filter(|l| *l == &expected).count() == 1`, that `expected` contains `127.0.0.1:2080` and `127.0.0.1:2081`, and that backend.log read from `stub.paths.logs_dir().join(\"backend.log\")` has `matches(&expected).count() == 1`. `v2ray_without_tun_logs_no_warning`: same stub and two candidates with the TUN-off settings; derives the sentence from the helper rather than a literal — `let expected = v2ray_tun_warning(&v2ray_settings(2080, 2081, \"127.0.0.1\", true)).expect(\"warning\");` — then asserts no collected line equals or contains `expected` and that backend.log does not contain it, so a later reword of the helper sentence cannot make this negative test pass vacuously.",
        "2.2b — add the emission block to `spawn_with` in crates/ui/src/connection.rs at the site spelled in this seam's summary."
      ],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings",
      "coder": "rust-coder"
    }
  ],
  "seams": [
    {
      "id": "warning-text",
      "tasks": [
        "1.1",
        "2.1"
      ],
      "summary": "The pure warning helper plus its first non-test consumer, landed together. Exact signature: `pub(super) fn v2ray_tun_warning(settings: &AppSettings) -> Option<String>` in crates/ui/src/connection.rs, placed next to `build_tun_runtime`, which already encodes the same v2ray-has-no-TUN rule. Takes the whole `&AppSettings` (not narrowed fields) to match `build_tun_runtime(&AppSettings, bool)` and because the caller in app.rs holds `self.settings` and the connection task holds `settings` verbatim. Body: `if !settings.tun.enabled || settings.backend.backend_type != BackendType::V2ray { return None; }` then `Some(format!(\"TUN is not supported by v2ray; only apps using the SOCKS proxy at {} or the HTTP proxy at {} are proxied\", listen_endpoint(&settings.listen_address, settings.socks_port), listen_endpoint(&settings.listen_address, settings.http_port)))`. Private sibling `fn listen_endpoint(addr: &str, port: u16) -> String` carries the IPv6 bracketing rule, spelled exactly: `if addr.parse::<std::net::Ipv6Addr>().is_ok() { format!(\"[{addr}]:{port}\") } else { format!(\"{addr}:{port}\") }` — parse-based, not `contains(':')`, so a hostname listen address is never bracketed. No bracketing helper exists anywhere in the workspace, so this is new code, not a reuse. Task 2.1 belongs in this seam, not in `surfacing`: `v2ray-rs-ui` has a lib target (crates/ui/src/lib.rs) and this chunk's verify runs `cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings`, which compiles the lib without cfg(test); a helper whose only caller is a test fails the chunk on dead_code. The toast call site is that caller. In `start_connection` (crates/ui/src/app.rs), immediately after the `tun_ipv6_unavailable` gate block and before the routing-rules load, insert `if let Some(warning) = crate::connection::v2ray_tun_warning(&self.settings) { self.show_toast(&warning); }` — no `return Err`, unlike the IPv6 gate one block above, so the connection still spawns. NO-RED-WAIVER: Rust workspace, no Rust test-writer agent in this pipeline; the coder writes the tests first inside its own chunk. NO-TESTER-WAIVER: same reason — tests and implementation land in one crate-local commit, tests authored before the implementation body. Task 2.1 additionally carries no automated test: `show_toast` needs a live ToastOverlay and a GTK main loop, so it is verified live by the user under task 3.2.",
      "contract": {
        "states": [
          "warn",
          "silent",
          "toast-shown",
          "toast-absent"
        ],
        "transitions": [
          {
            "input": "backend=V2ray, tun.enabled=true",
            "state": "warn",
            "effect": "Some(text naming SOCKS and HTTP endpoints)",
            "evidence": "spec scenario 'Connecting with v2ray while TUN is enabled warns' (openspec/changes/warn-v2ray-tun-unsupported/specs/tun-mode/spec.md:10-12)"
          },
          {
            "input": "backend=V2ray, tun.enabled=false",
            "state": "silent",
            "effect": "None",
            "evidence": "spec scenario 'No warning without the TUN flag' (spec.md:14-16)"
          },
          {
            "input": "backend=SingBox, tun.enabled=true",
            "state": "silent",
            "effect": "None",
            "evidence": "spec.md:14-16; TUN is real there, build_tun_runtime returns Some (connection.rs:440-469)"
          },
          {
            "input": "backend=Xray, tun.enabled=true",
            "state": "silent",
            "effect": "None",
            "evidence": "same as SingBox"
          },
          {
            "input": "backend=SingBox|Xray, tun.enabled=false",
            "state": "silent",
            "effect": "None",
            "evidence": "spec.md:14-16"
          },
          {
            "input": "listen_address parses as Ipv6Addr (e.g. `::1`)",
            "state": "warn",
            "effect": "text contains `[::1]:<socks>` and `[::1]:<http>`",
            "evidence": "task 1.1 'IPv6 listen address bracketed'"
          },
          {
            "input": "listen_address is IPv4 or a hostname",
            "state": "warn",
            "effect": "text contains `<addr>:<port>` unbracketed",
            "evidence": "task 1.1"
          },
          {
            "input": "Connect with backend=V2ray, tun.enabled=true",
            "state": "toast-shown",
            "effect": "set (one toast, connection still spawned)",
            "evidence": "app.rs tun_ipv6_unavailable gate shape one block above; spawn still reached; spec.md scenario \"Connecting with v2ray while TUN is enabled warns\""
          },
          {
            "input": "Connect with backend=V2ray, tun.enabled=false",
            "state": "toast-absent",
            "effect": "no-op",
            "evidence": "spec.md scenario \"No warning without the TUN flag\""
          },
          {
            "input": "Connect with backend=SingBox|Xray, tun.enabled=true",
            "state": "toast-absent",
            "effect": "no-op",
            "evidence": "spec.md scenario \"No warning without the TUN flag\""
          },
          {
            "input": "IPv6 preflight gate fires (tun_ipv6_unavailable)",
            "state": "toast-absent",
            "effect": "forced (start_connection returns Err before reaching the site)",
            "evidence": "app.rs: the tun_ipv6_unavailable block returns Err(TUN_IPV6_DISABLED)"
          }
        ],
        "forbidden": [
          "Returning Some for any backend other than V2ray — build_tun_runtime (connection.rs:440-447) already grants TUN to sing-box and xray; a warning there contradicts that guard.",
          "Returning Some when tun.enabled is false — the toggle state is the only trigger; nothing else in the pipeline is consulted.",
          "The helper mutating settings or clearing tun.enabled — non-goal, design.md:13.",
          "The helper returning Err or panicking — it is total over AppSettings; there is no failure path.",
          "The helper reading the running process, the host, or the filesystem — it is pure; the stub-backend test in the second seam depends on that.",
          "Showing the toast with a `return Err(..)` after it — that would block a valid v2ray SOCKS/HTTP session; design.md 'Warn, don't fail'.",
          "Writing a second copy of the text in app.rs instead of calling `v2ray_tun_warning` — one helper so toast and log stay identical.",
          "Clearing or mutating `settings.tun.enabled` at the toast site — non-goal."
        ],
        "seeding": [
          "warn: build `AppSettings { backend.backend_type: BackendType::V2ray, tun: TunConfig { enabled: true, ..default }, listen_address, socks_port, http_port }` by field assignment on `AppSettings::default()`, the same shape `tun_settings()` (connection.rs:1489-1497) and `singbox_settings()` (connection.rs:796-800) already use. The spec's 2080/2081 are NOT the defaults (defaults are 1080/1081, crates/core/src/models/settings.rs:223-224), so the test must set both ports explicitly.",
          "silent: the same construction with `tun.enabled = false`, or with `backend_type` set to `BackendType::SingBox` / `BackendType::Xray`.",
          "toast-shown / toast-absent: no automated seeding path — `show_toast` requires a live ToastOverlay and a GTK main loop; reached only by the manual run in task 3.2 (TUN on, backend switched to v2ray, Connect)."
        ]
      }
    },
    {
      "id": "surfacing",
      "tasks": [
        "2.2"
      ],
      "summary": "The once-per-connection log line, consuming the same helper so toast and log text cannot drift. In crates/ui/src/connection.rs, inside the `tokio::spawn` body of `spawn_with`, immediately after the `drop_strict_route` block and before `'candidates: for candidate in candidates`, emit through the exact mechanism the strict-route notice uses: `if let Some(warning) = v2ray_tun_warning(&settings) { if let Some(log) = &backend_log { log.append_line(\"warning\", &warning); } sender.emit(AppMsg::ProcessLogLine(generation, warning)); }`. The content is the helper's sentence verbatim with no added prefix, so the log line and the toast are the same text, which is what the spec's 'one line with the same warning' requires; the level lives in `append_line`'s stream tag, which renders as `<rfc3339> warning <sentence>`. (The STRICT_ROUTE_NOTICE precedent bakes `notice: ` into its const and also passes `\"notice\"` as the stream, so its on-disk line carries the label twice — not copied here.) `backend_log` is the single RotatingFileWriter that spans every candidate, so the line lands in backend.log; `AppMsg::ProcessLogLine(generation, ..)` is the logs-page stream. Once-per-connection is structural, not a flag: the emission is straight-line code in the task body executed before the candidate loop, reached exactly once per spawned task, and a task is spawned exactly once per Connect with a freshly bumped generation. The identical argument already holds for STRICT_ROUTE_NOTICE, whose once-ness is asserted over two candidates by `singbox_tun_without_ipv6_turns_strict_route_off_once`. NO-RED-WAIVER: Rust workspace, no Rust test-writer agent in this pipeline; the coder writes the tests first inside its own chunk. NO-TESTER-WAIVER: same reason — tests and implementation land in one crate-local commit, tests authored before the implementation body.",
      "contract": {
        "states": [
          "logged-once",
          "not-logged"
        ],
        "transitions": [
          {
            "input": "Connect with backend=V2ray, tun.enabled=true",
            "state": "logged-once",
            "effect": "set (exactly 1 line in the ProcessLogLine stream and exactly 1 occurrence in backend.log, regardless of candidate count)",
            "evidence": "connection.rs:142-150 precedent, assertion pattern connection.rs:1253-1257"
          },
          {
            "input": "Candidate 2..N starts after candidate 1 failed, backend=V2ray + TUN",
            "state": "logged-once",
            "effect": "no-op (no second emission; the site is outside the loop)",
            "evidence": "connection.rs:152 loop boundary"
          },
          {
            "input": "Connection task returns early on a queued Stop before the loop (connection.rs:106-109)",
            "state": "not-logged",
            "effect": "no-op (the Stop check precedes the emission site)",
            "evidence": "connection.rs:106-109"
          },
          {
            "input": "`RotatingFileWriter::open` failed, backend_log is None, backend=V2ray + TUN",
            "state": "logged-once",
            "effect": "set for the stream only, forced no-op for the file",
            "evidence": "connection.rs:131-135 makes backend_log Option and connection.rs:143 already guards with `if let Some(log)`"
          }
        ],
        "forbidden": [
          "Emitting the log line inside the `'candidates:` loop (connection.rs:152) — that yields one line per candidate and breaks the once-per-connection contract.",
          "Opening a second RotatingFileWriter for the warning — connection.rs:127-136 states one writer spans the connection.",
          "Clearing or mutating `settings.tun.enabled` at either site — non-goal, design.md:13.",
          "Raising the v2ray backend log level to surface the warning — non-goal, design.md:14.",
          "Adding a `warning: ` (or any) prefix to the emitted content — the log line must equal the helper's sentence so it matches the toast; the level is the `append_line` stream tag."
        ],
        "seeding": [
          "logged-once: `spawn_with(request(&stub, v2ray_settings(2080, 2081, \"127.0.0.1\", true), vec![candidate(\"203.0.113.1\"), candidate(\"203.0.113.2\")]), tx, capless_probe)` against `stub(r#\"exit 1\"#)` — TWO candidates, matching the precedent, because with one candidate a `count() == 1` assertion is satisfied by an emission site inside the loop too and proves nothing about once-ness. Every stub invocation fails, so `check_config` rejects each candidate and the task reaches a terminal `Error` immediately; `drain` then returns the collected lines. This is the only legal seeding path: the warning is produced by the running connection task, never by writing a line into the buffer or the file directly. v2ray never builds a TUN device (build_tun_runtime returns None), so no interface_name collision with concurrent tests.",
          "not-logged: the same stub and two candidates with `v2ray_settings(2080, 2081, \"127.0.0.1\", false)`.",
          "Never seed by constructing a ProcessLogLine message by hand or by appending to backend.log: both bypass the emission site under test."
        ]
      }
    }
  ],
  "requirements": [
    {
      "shall": "### Requirement: TUN mode availability per backend The system SHALL offer TUN mode only when the selected backend is sing-box or xray. For the v2ray backend, TUN SHALL be unavailable because v2ray-core has no native TUN inbound. When the user connects with the v2ray backend while TUN is enabled in settings, the connection SHALL proceed without TUN and the system SHALL warn the user, by a toast and by one line in the process log stream, that TUN is not supported by v2ray and only applications using the SOCKS or HTTP proxy at the configured listen address and ports are proxied. For xray, TUN SHALL additionally require Xray-core v26.1.13 or newer (the first release with the `tun` inbound); starting a TUN connection with an older xray SHALL fail before spawn with an error naming the installed and required versions. For xray versions in the range 26.1.13 through 26.6.22 — affected by the upstream TUN crash on quickly-closed connections (Xray-core #6364, fixed in 26.6.27) — the TUN start SHALL proceed but emit an advisory into the process log stream naming the installed version, the crash behavior, and the fixed version. When the installed xray version cannot be read or parsed, the start SHALL proceed and the process log SHALL contain a warning that the version could not be read and the minimum-version check was skipped.",
      "tests": [
        "v2ray_with_tun_warns_with_both_endpoints",
        "v2ray_without_tun_does_not_warn",
        "singbox_and_xray_with_tun_do_not_warn",
        "ipv6_listen_address_is_bracketed",
        "v2ray_tun_warning_logged_once_per_connection",
        "v2ray_without_tun_logs_no_warning",
        "task 3.2 live check (toast; GTK-bound, no automated vehicle)",
        "test_v2ray_never_emits_tun_even_when_enabled",
        "test_xray_no_tun_inbound_when_disabled",
        "test_xray_tun_inbound_emitted_when_enabled",
        "strict_route_row_sensitive_for_xray",
        "tun_start_fails_fast_on_pre_tun_xray_version",
        "xray_tun_minimum_version_comparison",
        "tun_start_warns_on_panic_affected_xray_version",
        "xray_tun_start_warns_on_unreadable_version"
      ]
    },
    {
      "shall": "- **THEN** the enable toggle SHALL be insensitive with an explanatory note, and no tun inbound SHALL be generated even if a stale `enabled` flag is persisted",
      "tests": [
        "test_v2ray_never_emits_tun_even_when_enabled",
        "test_xray_no_tun_inbound_when_disabled"
      ]
    },
    {
      "shall": "- **THEN** the connection SHALL start without TUN, a warning toast SHALL state that TUN is not supported by v2ray and only apps using SOCKS `127.0.0.1:2080` or HTTP `127.0.0.1:2081` are proxied, and the process log SHALL contain one line with the same warning",
      "tests": [
        "v2ray_with_tun_warns_with_both_endpoints",
        "v2ray_without_tun_does_not_warn",
        "singbox_and_xray_with_tun_do_not_warn",
        "ipv6_listen_address_is_bracketed",
        "v2ray_tun_warning_logged_once_per_connection",
        "v2ray_without_tun_logs_no_warning",
        "task 3.2 live check (toast; GTK-bound, no automated vehicle)"
      ]
    },
    {
      "shall": "- **THEN** no v2ray TUN warning SHALL be shown or logged",
      "tests": [
        "v2ray_without_tun_does_not_warn",
        "singbox_and_xray_with_tun_do_not_warn",
        "v2ray_without_tun_logs_no_warning"
      ]
    },
    {
      "shall": "- **THEN** the TUN enable toggle SHALL be available, subject to the capability gate",
      "tests": [
        "test_xray_tun_inbound_emitted_when_enabled",
        "strict_route_row_sensitive_for_xray"
      ]
    },
    {
      "shall": "- **THEN** the connection preflight SHALL fail with an error stating the installed version and the required minimum, without spawning the backend",
      "tests": [
        "tun_start_fails_fast_on_pre_tun_xray_version",
        "xray_tun_minimum_version_comparison"
      ]
    },
    {
      "shall": "- **THEN** the connection SHALL start normally and a warning log line SHALL appear in the process logs naming the installed version, the quickly-closed-connection crash, and 26.6.27 as the fixed version",
      "tests": [
        "tun_start_warns_on_panic_affected_xray_version"
      ]
    },
    {
      "shall": "- **THEN** the start SHALL continue and the process log SHALL contain a warning that the version could not be read and the minimum-version check was skipped",
      "tests": [
        "xray_tun_start_warns_on_unreadable_version"
      ]
    }
  ],
  "testHarness": [
    "stub(script) — crates/ui/src/connection.rs:723 — writes a 0o755 /bin/sh backend script into a TempDir, returns Stub { _tmp, paths: AppPaths::for_profile_in(AppProfile::Test, tmp), binary }; paths.ensure_dirs() already called. This is the stub-backend harness task 2.2 needs.",
    "strict_route_stub() — crates/ui/src/connection.rs:1184 — stub answering `version` with \"sing-box version 1.13.0\", `check` with exit 0, else `exec sleep 30`; a v2ray variant needs its own script (v2ray's config check arg is `test -c`).",
    "request(stub, settings, candidates) -> ConnectionRequest — crates/ui/src/connection.rs:765 — full request with ConfigWriter::new, pid/geodata paths, empty rules/subscriptions/manual_nodes, GENERATION = 7, host_has_ipv6 = true.",
    "connect_with(stub, settings, candidates, configure) — crates/ui/src/connection.rs:785 — relm4::channel + spawn_with(request(...), tx, configure); connect(...) at :756 is the identity-configure shorthand.",
    "capless_probe(mgr) — crates/ui/src/connection.rs:1197 — mgr.with_host_probe(HostProbe { getcap: /bin/true, helper: /bin/true }) so the capability gate does not require real caps; the strict-route notice tests pass it to spawn_with.",
    "drain(rx) -> (Option<ProcessState>, Vec<String>) — crates/ui/src/connection.rs:1159 — drains until channel close, returning the terminal state and every ProcessLogLine content; the exact assertion vehicle for a once-per-connection log line (see the `notices == 1` pattern at connection.rs:1253-1257).",
    "tun_settings() -> AppSettings — crates/ui/src/connection.rs:1489 — AppSettings::default() with tun.enabled = true; base for both the 1.1 unit tests and the 2.2 v2ray+TUN settings (backend_type must be set to V2ray, default is xray).",
    "singbox_settings() -> AppSettings — crates/ui/src/connection.rs:794 — AppSettings::default() with backend_type = SingBox; the negative-case analogue for a v2ray_settings() builder.",
    "strict_route_settings() — crates/ui/src/connection.rs:1190 — tun_settings() + SingBox + a distinct interface_name (\"v2rstest2\") so concurrent tests do not collide on the TUN device name.",
    "candidate(address) / xhttp_candidate(address) / node(address) — crates/ui/src/connection.rs:743 / 1204 / 1479 — ConnectionCandidate builders over a Shadowsocks or Vless-xhttp ProxyNode.",
    "next_state(rx) / assert_nothing_after_terminal(rx) / assert_error_terminal(terminal) — crates/ui/src/connection.rs:802, 816, ~1220 — terminal-state assertions with a 20s RECV_TIMEOUT.",
    "app.rs mod tests — crates/ui/src/app.rs:1808 — pure-function tests only (snapshot() at :1856 builds a RuntimeConfigSnapshot); there is NO App-instance or toast harness, so task 2.1's show_toast wiring has no unit-test vehicle and the task correctly asks for none."
  ],
  "floor": "timeout 10m cargo test --workspace -- --test-threads=4",
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
