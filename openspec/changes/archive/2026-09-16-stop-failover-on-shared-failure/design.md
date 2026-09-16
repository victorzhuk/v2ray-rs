## Context

- `crates/ui/src/connection.rs` `spawn`: a candidate fails in one of three places, each pushing `"{candidate_label}: {reason}"` into `failures` and continuing: config generation error, `start_with_connection` error, or `ProcessState::Error(msg)` after `wait_and_handle_exit` (crash budget exhausted). After the loop, `summarize_failures` reports `All candidates failed: <first 3>; ...`.
- Crash give-up text comes from `ProcessManager::handle_unexpected_exit`: `3 crashes within 60s: process exited with code 1: <last output line>`. The last line carries backend prefixes: sing-box `\x1b[31mFATAL\x1b[0m[0000] …` (the `[0000]` is seconds since start), xray `2026/09/14 10:37:35.309646 [Error] …`.
- Observed run: identical reason text on all 7 candidates.

## Goals / Non-Goals

**Goals:**
- A failure that is not about the node stops the attempt after the second candidate.

**Non-Goals:**
- Classifying individual error strings by backend (brittle, needs upkeep per backend version).
- Skipping the in-place crash respawns for deterministic startup FATALs. It would save a few more seconds but changes `ProcessManager` crash semantics; left for a separate change if still wanted.
- Changing automatic reconnect: it re-plans later and may hit the same wall. With this change each attempt costs two candidates, not all of them.

## Decisions

- **Compare consecutive failures, not a fixed error list.** Backend-agnostic, and a node-independent failure repeats by construction. Two in a row is the lowest count that tells one bad node apart from a shared cause. Alternative rejected: matching known phrases such as `starting TUN interface`, which covers only the case already seen.
- **Normalize before comparing.** Strip ANSI SGR escapes, a leading `YYYY/MM/DD HH:MM:SS(.frac)` timestamp, and bracketed all-digit tokens such as `[0000]`; then replace every occurrence of the candidate's label, address, and port with a placeholder. Compare the resulting strings for equality. Anything that still differs (dial errors naming the server, TLS errors naming the SNI) keeps failing over.
- **Summary wording.** `Connection failed on consecutive nodes with the same error (not node-specific): <reason>` where `<reason>` is the ANSI-stripped reason of the last failure. The existing per-node summary keeps its format for mixed failures, also ANSI-stripped.

## Risks / Trade-offs

- [Two genuinely broken nodes fail identically, e.g. a provider-wide outage whose message names no host, while a later node would work] → the user sees the reason and can connect directly to another node; automatic reconnect re-plans. Acceptable next to 40 s of futile retries hiding a host fault.
- [Normalization misses a volatile token, so identical failures look different] → falls back to today's behavior (full failover), never worse.

## Implementation plan

Tier standard, mode existing-service-strict, lenses `spec` + `quality`. Rust stack: both seams carry `NO-RED-WAIVER:` / `NO-TESTER-WAIVER:`; tests are each coder's first task and a chunk closes on its verify command. Both chunks edit `crates/ui/src/connection.rs`, so they are serial in one worktree.

**c1-failure-key** — tasks 1.1 and 1.2, coder `rust-coder`.
- `fn strip_ansi(s: &str) -> String` and pure `fn failure_key(reason: &str, label: &str, address: &str, port: u16) -> String`: strip ANSI SGR escapes, a leading `YYYY/MM/DD HH:MM:SS(.frac)` timestamp and bracketed all-digit tokens, then mask the candidate's label, address and port. Hand-rolled — `regex` is not a dependency of any workspace crate, and a dependency for ~60 lines of char scanning is not warranted.
- `struct CandidateFailure { label, reason, key }` replaces the `Vec<String>` failure list, so the loop can compare keys while the summary prints reasons. `summarize_failures` keeps its format and gains ANSI stripping.
- `fn repeats_previous(failures: &[CandidateFailure]) -> bool` (len ≥ 2 and the last two keys equal) lands here too, with `repeats_previous_needs_two_equal_keys`: without a reader, `key` would trip dead code. Only the loop wiring is left to c2.
- Tests: `failure_key_ignores_ansi_and_elapsed_counter`, `failure_key_ignores_leading_timestamp`, `failure_key_masks_candidate_identity`, `failure_key_keeps_unrelated_hosts_distinct`, `failure_key_keeps_midstring_timestamp`, `summarize_failures_strips_ansi`, `repeats_previous_needs_two_equal_keys`.
- Verify: tests only — `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 failure_key && … repeats_previous && … connection::tests`. Clippy runs at c2 and in the floor, because `repeats_previous` has no non-test caller until the loop is wired; silencing dead code with an attribute is forbidden.

**c2-early-stop** — tasks 2.1 and 2.2, prev c1-failure-key, coder `rust-coder`.
- Label the candidate loop `'candidates` and `break 'candidates` when `repeats_previous(&failures)` holds right after a failure is pushed. No teardown, report or return inside the loop: the post-loop path owns both, byte for byte as the exhausted-list path does today, so parked-manager and Stop semantics are unchanged.
- After the loop: when `repeats_previous` still holds, report `Connection failed on consecutive nodes with the same error (not node-specific): <reason>` with the ANSI-stripped reason of the last failure; otherwise `summarize_failures`.
- Tests seed a chosen failure through the config-check path (the stub's `check` branch prints the text to stderr and exits 1) and count launches with a marker file the stub appends to, in a second `TempDir` the test owns. Each stub keeps the existing `version` branch verbatim and that branch never appends. Seeded messages stay under 200 characters, the reason-truncation cap.
  - `repeated_failure_stops_after_two_candidates`: 3 candidates, same FATAL with a different `[000N]` counter → marker has 2 lines, one terminal `Error` with the new wording, no `\x1b`, generated config holds candidate 2 and not candidate 3.
  - `differing_failures_try_every_candidate`: 3 distinct unrelated-host failures → 3 launches, the existing `All candidates failed: …` summary, ANSI-stripped.
- Verify: `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 connection::tests && timeout 10m cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings`.

**Floor (3.1):** `timeout 10m cargo test --workspace -- --test-threads=4 && timeout 10m cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings`.

**Manual (3.2):** a live reproduction. The original reproducer no longer occurs — `fix-singbox-tun-without-ipv6` is archived — so it needs a TUN setting that fails at startup. Note that a crash-budget failure writes one session record per launch including respawns, so the check is two distinct `node=` values, not two session records.

Plan review: zarchitect, 3 rounds, pass. Round 1: the c2 site text contradicted the seam contract on teardown, and `CandidateFailure.key` had no reader before clippy. Round 2: `repeats_previous` moved into c1 but was still unused in the lib target. Round 3 confirmed the fix — clippy staged to c2 and the floor.

## Plan appendix

```json
{
  "v": 2,
  "change": "stop-failover-on-shared-failure",
  "baseSha": "d9aa18966745ff67d7bf3fc918bac34e4d06c4db",
  "generatedAt": "2026-09-16T05:26:38.992Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "estimateHours": 1.8,
  "chunks": [
    {
      "id": "c1-failure-key",
      "taskIds": [
        "1.1",
        "1.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": false,
      "seam": "failure-key",
      "shard": "",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "failure_key (new pure fn, next to summarize_failures)",
          "anchor": "fn summarize_failures(failures: &[String]) -> String {",
          "change": "Add pure `failure_key(reason: &str, label: &str, address: &str, port: u16) -> String` above/below summarize_failures: strip ANSI SGR (ESC '[' ... 'm'), a leading YYYY/MM/DD HH:MM:SS(.frac) timestamp, bracketed all-digit tokens ([0000]), then replace every occurrence of label/address/port with a placeholder. No regex crate exists in the workspace (zero hits for `regex` in any Cargo.toml) — hand-written char scan."
        },
        {
          "task": "1.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests::failure_key unit tests",
          "anchor": "    fn forwarder_relays_only_nonterminal_states() {",
          "change": "Add #[test] rows next to the existing pure-fn tests: sing-box FATAL [0000] vs [0001] equal, xray lines with differing `2026/09/14 10:37:35.309646` prefixes equal, TLS errors naming each candidate's own address equal, TLS error naming an unrelated host differs."
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with — failures accumulator",
          "anchor": "        let mut failures = Vec::new();",
          "change": "Failures carry label + ANSI-stripped reason separately (e.g. Vec<(String,String)> or a small struct with the key), so the loop can compare keys and the summary can still print `{label}: {reason}`."
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with — failure site 1 (config generation)",
          "anchor": "                        failures.push(format!(\"{candidate_label}: config generation failed: {e}\"));",
          "change": "Push label + reason `config generation failed: {e}` (stripped) and compute its key before `continue`."
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with — failure site 2 (start_with_connection error, non host-level)",
          "anchor": "                    failures.push(format!(\"{candidate_label}: {e}\"));",
          "change": "Same split; note this arm also does `parked = Some(mgr); continue;`. The host-level arm above it (`if e.is_host_level()`) returns its own Error and is untouched."
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with — failure site 3 (crash give-up after wait_and_handle_exit)",
          "anchor": "                                failures.push(format!(\"{candidate_label}: {msg}\"));",
          "change": "Same split; `msg` is the ProcessManager give-up text and is the ANSI-carrying one. This arm `break`s out of the supervise loop into `halt(...)`/`parked = Some(mgr)`."
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "summarize_failures",
          "anchor": "        return \"All candidates failed\".into();",
          "change": "Signature changes to take the new failure records; keeps `All candidates failed: <first 3 joined by \"; \">` and `; ...` when len>3, rendering `{label}: {reason}` with ANSI already stripped. Existing assertions to keep passing: `msg.starts_with(\"All candidates failed\")`, `msg.contains(\"203.0.113.1: 3 crashes\")`, `msg.contains(\"203.0.113.3: config rejected\")`."
        }
      ],
      "contract": {
        "states": [
          "CandidateFailure.label — candidate_label at the failure site (remark, else node address)",
          "CandidateFailure.reason — failure text after strip_ansi, no label prefix",
          "CandidateFailure.key — failure_key(reason, label, address, port)",
          "failures: Vec<CandidateFailure> — one entry per failed candidate, push order = candidate order"
        ],
        "transitions": [
          {
            "input": "reason containing CSI SGR escapes, e.g. \"\\u001b[31mFATAL\\u001b[0m[0000] start service: ...\"",
            "state": "strip_ansi output",
            "effect": "set",
            "evidence": "requirement: 'Failure text reported to the user SHALL NOT contain terminal color escape sequences' (specs/process-lifecycle). Scanner: on ESC followed by '[', drop bytes while in 0x30..=0x3F, then 0x20..=0x2F, then one final byte 0x40..=0x7E; a lone ESC is dropped; all other chars copied."
          },
          {
            "input": "normalization step order inside failure_key",
            "state": "key",
            "effect": "forced",
            "evidence": "design.md Decisions 'Normalize before comparing'. Order is fixed: (1) strip_ansi, (2) trim, (3) strip one leading timestamp, (4) bracketed all-digit tokens -> [N], (5) label -> <node>, (6) address -> <node>, (7) port -> <port>. Step 4 precedes 6/7 so [0000] can never be eaten by the port rule."
          },
          {
            "input": "leading \"2026/09/14 10:37:35.309646 \" (xray) or \"2026/09/14 10:37:35 \"",
            "state": "key without that prefix",
            "effect": "clear",
            "evidence": "design.md Decisions; shape check at fixed offsets: DDDD/DD/DD SPACE DD:DD:DD, optional '.' + 1..=9 digits, then all following spaces consumed. Only at position 0, only once."
          },
          {
            "input": "timestamp-shaped text not at position 0",
            "state": "key",
            "effect": "no-op",
            "evidence": "design.md Decisions: only a leading timestamp is volatile; mid-string dates are part of the message."
          },
          {
            "input": "bracketed all-digit token \"[0000]\", \"[0001]\", \"[12]\"",
            "state": "key contains \"[N]\"",
            "effect": "set",
            "evidence": "tasks.md 1.1 'sing-box [0000] vs [0001] FATAL equal'. Body must be non-empty and all ASCII digits; \"[tun-in]\" and \"[]\" are left untouched."
          },
          {
            "input": "occurrence of candidate label (len >= 3) anywhere in the reason",
            "state": "key contains \"<node>\"",
            "effect": "set",
            "evidence": "design.md Decisions 'replace every occurrence of the candidate's label, address, and port with a placeholder'. Length floor 3 keeps a two-letter remark such as \"US\" from masking unrelated substrings."
          },
          {
            "input": "occurrence of candidate address (len >= 3)",
            "state": "key contains \"<node>\"",
            "effect": "set",
            "evidence": "design.md Decisions. Applied after the label pass; already-substituted text no longer matches, so the two passes are idempotent when label == address (the remark-less default, connection.rs candidate_label)."
          },
          {
            "input": "candidate port digits bounded on both sides by a char that is neither an ASCII digit nor '.'",
            "state": "key contains \"<port>\"",
            "effect": "set",
            "evidence": "design.md Decisions. The boundary rule stops \"443\" from being clipped out of \"14430\" or \"10.4.43.1\"."
          },
          {
            "input": "two candidates' reasons that differ only in ANSI, leading timestamp, bracketed counters, own label/address/port",
            "state": "equal keys",
            "effect": "set",
            "evidence": "spec scenario 'Same failure on consecutive candidates stops failover'"
          },
          {
            "input": "reasons naming unrelated hosts (neither candidate's own address)",
            "state": "different keys",
            "effect": "no-op",
            "evidence": "spec scenario 'Node-specific failures keep failing over'; tasks.md 1.1 last bullet"
          },
          {
            "input": "a failure pushed at any of the three sites",
            "state": "failures gains one CandidateFailure",
            "effect": "set",
            "evidence": "crates/ui/src/connection.rs:190 (config generation), :273 (start_with_connection error, non host-level), :332 (crash give-up after wait_and_handle_exit)"
          },
          {
            "input": "summarize_failures(&[CandidateFailure])",
            "state": "\"All candidates failed: <up to 3 joined by '; '>\" plus \"; ...\" when len > 3",
            "effect": "no-op",
            "evidence": "crates/ui/src/connection.rs:482-499 — format unchanged; each entry is now rendered as format!(\"{label}: {reason}\") from the stored fields, and reason is already ANSI-stripped"
          }
        ],
        "forbidden": [
          "failure_key must not allocate a regex or pull a new crate dependency into crates/ui",
          "failure_key must not be fallible or panic on non-ASCII input — it operates on chars, never byte slicing inside a multi-byte char",
          "no failure text reaching ProcessState::Error may contain U+001B",
          "the failures list must not keep the pre-joined \"label: reason\" String as its only form (task 1.2 requires label and reason separately)"
        ],
        "seeding": [
          "failure_key and strip_ansi are pure: unit tests call them directly with literal strings; no fixture, no filesystem",
          "CandidateFailure is constructed only inside the candidate loop from candidate_label, the error text, candidate.node.address(), candidate.node.port(); tests reach it through the stub-backend path, never by hand-building the vector"
        ],
        "budgets": [
          "leading-timestamp fraction: 1..=9 digits",
          "label/address replacement floor: 3 chars",
          "summary preview: 3 entries (unchanged)"
        ],
        "decisions": [
          "c1 also lands `fn repeats_previous(failures: &[CandidateFailure]) -> bool` (true when len >= 2 and the last two `key` values are equal) and its unit test `repeats_previous_needs_two_equal_keys`, so `CandidateFailure.key` has a reader in this chunk: without one, `cargo clippy -- -D warnings` fails on `field `key` is never read`. Only the loop wiring stays in c2.",
          "c1 runs tests only: `repeats_previous` has no non-test caller until c2 wires the loop, so `clippy --all-targets -- -D warnings` would fail on dead_code for half a feature. Clippy runs at c2 and in the floor. The coder must not add an allow attribute to silence it."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "1.1",
        "1.2"
      ],
      "redTests": [],
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 failure_key && timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 repeats_previous && timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 connection::tests",
      "coder": "rust-coder"
    },
    {
      "id": "c2-early-stop",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "prev": "c1-failure-key",
      "sharedPkg": "crates/ui/src/connection.rs",
      "parallel": false,
      "seam": "early-stop",
      "shard": "",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with — candidate loop early stop",
          "anchor": "        for candidate in candidates {",
          "change": "label the loop `'candidates: for candidate in candidates {` and, immediately after each failure push, `if repeats_previous(&failures) { break 'candidates; }`. No teardown, no report and no return inside the loop — the post-loop path (unchanged) owns both."
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn_with — terminal summary report",
          "anchor": "        report(ProcessState::Error(summarize_failures(&failures)), None);",
          "change": "choose the message here: when `repeats_previous(&failures)` holds, report `ProcessState::Error(format!(\"Connection failed on consecutive nodes with the same error (not node-specific): {reason}\", ...))` with the ANSI-stripped reason of the last failure; otherwise `summarize_failures(&failures)` as today. Teardown around the report is unchanged."
        },
        {
          "task": "2.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests::<new> same failure on consecutive candidates stops failover",
          "anchor": "    async fn last_candidate_failure_reports_one_error() {",
          "change": "New #[tokio::test(flavor = \"multi_thread\")] after it: 3 candidates, stub whose `check` succeeds and whose run prints an identical ANSI FATAL and exits (or whose `check` fails identically) — assert exactly 2 launches, one terminal Error starting with `Connection failed on consecutive nodes with the same error`, and `!msg.contains('\\u{1b}')`."
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests::<new> differing failures keep failing over",
          "anchor": "    async fn last_candidate_failure_reports_one_error() {",
          "change": "New stub test: 3 candidates whose stub keys off `grep -q <addr> \"$3\"` to emit a different reason per candidate → all 3 launched, terminal Error keeps `All candidates failed: ...` shape and carries no ESC."
        },
        {
          "task": "2.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::handle_unexpected_exit",
          "anchor": "                        \"{MAX_CRASHES} crashes within {CRASH_WINDOW:?}: {msg}\"",
          "change": "Read-only reference; not modified. Exact give-up text: `3 crashes within 60s: ` + msg, where msg = `process exited with code {code}` (or `process killed by signal`) + `: {last_output_line}` when a last line exists (crates/process/src/manager.rs:803-809). MAX_CRASHES = 3 (manager.rs:37), CRASH_WINDOW = Duration::from_secs(60) (manager.rs:38, Debug-printed as `60s`). last_output_line takes the last non-empty stderr line (else stdout) from the final 50 buffered lines and passes it through truncate_reason, which only replaces \\n/\\r and truncates — it does NOT strip ANSI (manager.rs:907-918, 941-951)."
        }
      ],
      "contract": {
        "states": [
          "parked: Option<ProcessManager> — unchanged",
          "failures: Vec<CandidateFailure>",
          "terminal message emitted by the single post-loop report(ProcessState::Error(..), None)"
        ],
        "transitions": [
          {
            "input": "candidate N fails (any of the three sites) and failures.len() >= 2 and last two keys are equal",
            "state": "loop exits via `break 'candidates`",
            "effect": "forced",
            "evidence": "spec scenario 'Same failure on consecutive candidates stops failover' — no third candidate started. Check placement: config-generation site after the push, replacing `continue` (connection.rs:190-192); start-error site after `parked = Some(mgr);` and before `continue` (connection.rs:273-275); crash site after the inner loop's `break`, after `halt(state_forwarder)`, `halt(log_forwarder)` and `parked = Some(mgr)` (connection.rs:332, 344-346) so the forwarders are always stopped before the terminal state, exactly as today."
          },
          {
            "input": "candidate N fails and the previous key differs (or it is the first failure)",
            "state": "loop continues to candidate N+1",
            "effect": "no-op",
            "evidence": "spec scenario 'Node-specific failures keep failing over'; proposal.md 'Failover for differing, node-specific failures is unchanged'"
          },
          {
            "input": "single candidate that fails (direct connect)",
            "state": "loop ends naturally, summarize_failures wording",
            "effect": "no-op",
            "evidence": "proposal.md 'so is a direct connect (single candidate)' — repeats_previous is false below 2 entries"
          },
          {
            "input": "loop exhausted and the last two failures share a key",
            "state": "not-node-specific wording",
            "effect": "set",
            "evidence": "spec requirement text says 'two consecutive candidates ... fail with the same reason', with no exception for the final pair; the post-loop branch therefore uses the same predicate for both exits"
          },
          {
            "input": "loop exhausted with mixed failures",
            "state": "\"All candidates failed: ...\" (ANSI-stripped)",
            "effect": "no-op",
            "evidence": "spec scenario 'Last candidate fails'; crates/ui/src/connection.rs:349"
          },
          {
            "input": "early stop reached",
            "state": "exactly one terminal Error, text `Connection failed on consecutive nodes with the same error (not node-specific): <reason>` where <reason> is failures.last().reason (already ANSI-stripped)",
            "effect": "set",
            "evidence": "design.md Decisions 'Summary wording'; tasks.md 2.1"
          },
          {
            "input": "ConnectionCmd::Stop arriving before/while a candidate runs",
            "state": "Stopped reported, parked shut down, task returns",
            "effect": "no-op",
            "evidence": "crates/ui/src/connection.rs:106-109, :153-159, :233-244, :251-255, :318-323 — no line of these paths is touched"
          },
          {
            "input": "host-level start error (ProcessError::is_host_level)",
            "state": "parked shut down, host error reported verbatim, return",
            "effect": "no-op",
            "evidence": "crates/ui/src/connection.rs:258-272 — precedes the failures push and is unchanged"
          }
        ],
        "forbidden": [
          "reporting more than one terminal state per connection attempt",
          "shutting down or dropping the parked manager on the early-stop path (today's exhausted-list path does not, and the change must not alter routing/TUN teardown semantics)",
          "the early-stop Error text containing a per-node list, a node name, or U+001B",
          "starting a third candidate after two identical failures"
        ],
        "seeding": [
          "A candidate failure with a chosen message is seeded through the config-check path: the stub script answers `[ \"$1\" = check ]` by printing the chosen text to stderr and exiting 1. ProcessManager::check_config takes the last non-empty stderr line and returns ProcessError::ConfigCheck(reason) (crates/process/src/manager.rs:667-713), which is not host-level (manager.rs:2028 table), so spawn_with pushes it at connection.rs:273. This costs no crash budget — the 3-crash path is ~6s per candidate (CRASH_RESTART_DELAY 2s, MAX_CRASHES 3, manager.rs:36-37, 846).",
          "Launch count is observed by a marker file the stub appends to in its `check` branch. The path must exist before stub() is called, so the test creates its own `let launches = tempfile::tempdir().unwrap();` and formats the script with `launches.path().join(\"launched\").display()`; stub() then writes that script verbatim (crates/ui/src/connection.rs:521-533 — stub takes the script as &str and only prefixes `#!/bin/sh`). Count = lines in the marker file. The `version` branch must not append, so the count is candidates attempted, not process invocations. This mechanism does not exist at HEAD; the test creates it — no production change is needed for it.",
          "Corroborating observable, already used at HEAD: the generated config on disk is the last candidate written (connection.rs:186-193; asserted the same way in host_level_failure_stops_the_candidate_loop, connection.rs:921-927). After an early stop it must contain candidate 2's address and not candidate 3's.",
          "Per-candidate message variation is keyed off the config: `grep -q <address> \"$3\"` inside the stub, the pattern already used at connection.rs:663 and :695."
        ],
        "budgets": [
          "2.1: exactly 2 marker lines out of 3 candidates",
          "2.2: exactly 3 marker lines out of 3 candidates",
          "exactly 1 terminal state per test (drain() already asserts none follows, connection.rs:717-740)",
          "test wall clock: both tests take the config-check path, so no candidate spends the ~6s crash budget; RECV_TIMEOUT 20s per message is untouched"
        ],
        "decisions": [
          "Implementation form is fixed: label the candidate loop and `break 'candidates` when `repeats_previous(&failures)` holds right after a failure is pushed. The message is chosen after the loop: the not-node-specific wording when `repeats_previous(&failures)` still holds, otherwise `summarize_failures`. Teardown on that path is byte-for-byte today's exhausted-list path — never shut down or drop the parked manager on the early-stop path, never an extra `return` inside the loop.",
          "Each new stub script starts with the existing version branch verbatim (`[ \"$1\" = version ] && echo \"sing-box version 1.13.0\" && exit 0`) and that branch must not append to the launch-marker file; only the `check` branch appends and then exits non-zero with the seeded message on stderr.",
          "Keep every seeded failure message under 200 characters: check_config runs it through truncate_reason (REASON_MAX_CHARS = 200, crates/process/src/manager.rs), so a longer message loses the asserted tail."
        ]
      },
      "redTasks": [],
      "codeTasks": [
        "2.1",
        "2.2"
      ],
      "redTests": [],
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 connection::tests && timeout 10m cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings",
      "coder": "rust-coder"
    }
  ],
  "seams": [
    {
      "id": "failure-key",
      "tasks": [
        "1.1",
        "1.2"
      ],
      "summary": "NO-RED-WAIVER: rust stack, no test-writer agent; tests are the first codeTask. NO-TESTER-WAIVER: rust stack; chunk closes on its verify command. Pure normalization `fn failure_key(reason: &str, label: &str, address: &str, port: u16) -> String` in crates/ui/src/connection.rs plus `fn strip_ansi(s: &str) -> String`, and a `struct CandidateFailure { label: String, reason: String, key: String }` replacing the `Vec<String>` failure list so the loop compares keys and the summary prints reasons. Hand-rolled scanning, no new dependency: `regex` is not a direct dependency of any workspace crate (crates/ui/Cargo.toml has none; root Cargo.toml [workspace.dependencies] has none; Cargo.lock carries it only transitively). ANSI SGR stripping and fixed-shape timestamp/bracket scanning are ~60 lines of char-class code; a direct regex dep for that is not warranted.",
      "contract": {
        "states": [
          "CandidateFailure.label — candidate_label at the failure site (remark, else node address)",
          "CandidateFailure.reason — failure text after strip_ansi, no label prefix",
          "CandidateFailure.key — failure_key(reason, label, address, port)",
          "failures: Vec<CandidateFailure> — one entry per failed candidate, push order = candidate order"
        ],
        "transitions": [
          {
            "input": "reason containing CSI SGR escapes, e.g. \"\\u001b[31mFATAL\\u001b[0m[0000] start service: ...\"",
            "state": "strip_ansi output",
            "effect": "set",
            "evidence": "requirement: 'Failure text reported to the user SHALL NOT contain terminal color escape sequences' (specs/process-lifecycle). Scanner: on ESC followed by '[', drop bytes while in 0x30..=0x3F, then 0x20..=0x2F, then one final byte 0x40..=0x7E; a lone ESC is dropped; all other chars copied."
          },
          {
            "input": "normalization step order inside failure_key",
            "state": "key",
            "effect": "forced",
            "evidence": "design.md Decisions 'Normalize before comparing'. Order is fixed: (1) strip_ansi, (2) trim, (3) strip one leading timestamp, (4) bracketed all-digit tokens -> [N], (5) label -> <node>, (6) address -> <node>, (7) port -> <port>. Step 4 precedes 6/7 so [0000] can never be eaten by the port rule."
          },
          {
            "input": "leading \"2026/09/14 10:37:35.309646 \" (xray) or \"2026/09/14 10:37:35 \"",
            "state": "key without that prefix",
            "effect": "clear",
            "evidence": "design.md Decisions; shape check at fixed offsets: DDDD/DD/DD SPACE DD:DD:DD, optional '.' + 1..=9 digits, then all following spaces consumed. Only at position 0, only once."
          },
          {
            "input": "timestamp-shaped text not at position 0",
            "state": "key",
            "effect": "no-op",
            "evidence": "design.md Decisions: only a leading timestamp is volatile; mid-string dates are part of the message."
          },
          {
            "input": "bracketed all-digit token \"[0000]\", \"[0001]\", \"[12]\"",
            "state": "key contains \"[N]\"",
            "effect": "set",
            "evidence": "tasks.md 1.1 'sing-box [0000] vs [0001] FATAL equal'. Body must be non-empty and all ASCII digits; \"[tun-in]\" and \"[]\" are left untouched."
          },
          {
            "input": "occurrence of candidate label (len >= 3) anywhere in the reason",
            "state": "key contains \"<node>\"",
            "effect": "set",
            "evidence": "design.md Decisions 'replace every occurrence of the candidate's label, address, and port with a placeholder'. Length floor 3 keeps a two-letter remark such as \"US\" from masking unrelated substrings."
          },
          {
            "input": "occurrence of candidate address (len >= 3)",
            "state": "key contains \"<node>\"",
            "effect": "set",
            "evidence": "design.md Decisions. Applied after the label pass; already-substituted text no longer matches, so the two passes are idempotent when label == address (the remark-less default, connection.rs candidate_label)."
          },
          {
            "input": "candidate port digits bounded on both sides by a char that is neither an ASCII digit nor '.'",
            "state": "key contains \"<port>\"",
            "effect": "set",
            "evidence": "design.md Decisions. The boundary rule stops \"443\" from being clipped out of \"14430\" or \"10.4.43.1\"."
          },
          {
            "input": "two candidates' reasons that differ only in ANSI, leading timestamp, bracketed counters, own label/address/port",
            "state": "equal keys",
            "effect": "set",
            "evidence": "spec scenario 'Same failure on consecutive candidates stops failover'"
          },
          {
            "input": "reasons naming unrelated hosts (neither candidate's own address)",
            "state": "different keys",
            "effect": "no-op",
            "evidence": "spec scenario 'Node-specific failures keep failing over'; tasks.md 1.1 last bullet"
          },
          {
            "input": "a failure pushed at any of the three sites",
            "state": "failures gains one CandidateFailure",
            "effect": "set",
            "evidence": "crates/ui/src/connection.rs:190 (config generation), :273 (start_with_connection error, non host-level), :332 (crash give-up after wait_and_handle_exit)"
          },
          {
            "input": "summarize_failures(&[CandidateFailure])",
            "state": "\"All candidates failed: <up to 3 joined by '; '>\" plus \"; ...\" when len > 3",
            "effect": "no-op",
            "evidence": "crates/ui/src/connection.rs:482-499 — format unchanged; each entry is now rendered as format!(\"{label}: {reason}\") from the stored fields, and reason is already ANSI-stripped"
          }
        ],
        "forbidden": [
          "failure_key must not allocate a regex or pull a new crate dependency into crates/ui",
          "failure_key must not be fallible or panic on non-ASCII input — it operates on chars, never byte slicing inside a multi-byte char",
          "no failure text reaching ProcessState::Error may contain U+001B",
          "the failures list must not keep the pre-joined \"label: reason\" String as its only form (task 1.2 requires label and reason separately)"
        ],
        "seeding": [
          "failure_key and strip_ansi are pure: unit tests call them directly with literal strings; no fixture, no filesystem",
          "CandidateFailure is constructed only inside the candidate loop from candidate_label, the error text, candidate.node.address(), candidate.node.port(); tests reach it through the stub-backend path, never by hand-building the vector"
        ],
        "budgets": [
          "leading-timestamp fraction: 1..=9 digits",
          "label/address replacement floor: 3 chars",
          "summary preview: 3 entries (unchanged)"
        ]
      },
      "codeTasks": [
        "Add unit tests in crates/ui/src/connection.rs tests module: failure_key_ignores_ansi_and_elapsed_counter (sing-box FATAL with [0000] vs [0001], different labels/addresses -> equal keys, key contains no U+001B), failure_key_ignores_leading_timestamp (two xray \"2026/09/14 10:37:35.309646 [Error] ...\" lines with different times and fractions -> equal), failure_key_masks_candidate_identity (TLS error naming each candidate's own address:port -> equal), failure_key_keeps_unrelated_hosts_distinct (same TLS error naming two unrelated hosts -> different), failure_key_keeps_midstring_timestamp (a date after the first char survives), summarize_failures_strips_ansi (a CandidateFailure whose reason came from ANSI text renders clean and keeps the \"All candidates failed: ...\" shape)",
        "Implement strip_ansi, failure_key, CandidateFailure { label, reason, key } with a constructor taking (label: &str, reason: String, address: &str, port: u16) that strips ANSI once and computes the key, plus a report() -> String rendering \"{label}: {reason}\"",
        "Change failures to Vec<CandidateFailure>; update the three push sites (connection.rs:190, :273, :332) to build CandidateFailure from candidate_label + the raw error text + candidate.node.address() + candidate.node.port(); change summarize_failures to take &[CandidateFailure] and join report() values, format otherwise unchanged",
        "Keep last_candidate_failure_reports_one_error passing unmodified (crates/ui/src/connection.rs:693)"
      ]
    },
    {
      "id": "early-stop",
      "tasks": [
        "2.1",
        "2.2",
        "3.1",
        "3.2"
      ],
      "summary": "NO-RED-WAIVER: rust stack, no test-writer agent; tests are the first codeTask. NO-TESTER-WAIVER: rust stack; chunk closes on its verify command. The candidate `for` loop in spawn_with gets the label 'candidates and a single stop decision: whenever a failure was just pushed, `repeats_previous(&failures)` (last two keys equal, len >= 2) breaks out of the loop; the message chosen after the loop is the new not-node-specific wording when repeats_previous still holds, otherwise summarize_failures. Teardown on that path is byte-for-byte today's exhausted-list path (parked manager left as it is, no extra shutdown), so parked handling and Stop semantics are untouched.",
      "contract": {
        "states": [
          "parked: Option<ProcessManager> — unchanged",
          "failures: Vec<CandidateFailure>",
          "terminal message emitted by the single post-loop report(ProcessState::Error(..), None)"
        ],
        "transitions": [
          {
            "input": "candidate N fails (any of the three sites) and failures.len() >= 2 and last two keys are equal",
            "state": "loop exits via `break 'candidates`",
            "effect": "forced",
            "evidence": "spec scenario 'Same failure on consecutive candidates stops failover' — no third candidate started. Check placement: config-generation site after the push, replacing `continue` (connection.rs:190-192); start-error site after `parked = Some(mgr);` and before `continue` (connection.rs:273-275); crash site after the inner loop's `break`, after `halt(state_forwarder)`, `halt(log_forwarder)` and `parked = Some(mgr)` (connection.rs:332, 344-346) so the forwarders are always stopped before the terminal state, exactly as today."
          },
          {
            "input": "candidate N fails and the previous key differs (or it is the first failure)",
            "state": "loop continues to candidate N+1",
            "effect": "no-op",
            "evidence": "spec scenario 'Node-specific failures keep failing over'; proposal.md 'Failover for differing, node-specific failures is unchanged'"
          },
          {
            "input": "single candidate that fails (direct connect)",
            "state": "loop ends naturally, summarize_failures wording",
            "effect": "no-op",
            "evidence": "proposal.md 'so is a direct connect (single candidate)' — repeats_previous is false below 2 entries"
          },
          {
            "input": "loop exhausted and the last two failures share a key",
            "state": "not-node-specific wording",
            "effect": "set",
            "evidence": "spec requirement text says 'two consecutive candidates ... fail with the same reason', with no exception for the final pair; the post-loop branch therefore uses the same predicate for both exits"
          },
          {
            "input": "loop exhausted with mixed failures",
            "state": "\"All candidates failed: ...\" (ANSI-stripped)",
            "effect": "no-op",
            "evidence": "spec scenario 'Last candidate fails'; crates/ui/src/connection.rs:349"
          },
          {
            "input": "early stop reached",
            "state": "exactly one terminal Error, text `Connection failed on consecutive nodes with the same error (not node-specific): <reason>` where <reason> is failures.last().reason (already ANSI-stripped)",
            "effect": "set",
            "evidence": "design.md Decisions 'Summary wording'; tasks.md 2.1"
          },
          {
            "input": "ConnectionCmd::Stop arriving before/while a candidate runs",
            "state": "Stopped reported, parked shut down, task returns",
            "effect": "no-op",
            "evidence": "crates/ui/src/connection.rs:106-109, :153-159, :233-244, :251-255, :318-323 — no line of these paths is touched"
          },
          {
            "input": "host-level start error (ProcessError::is_host_level)",
            "state": "parked shut down, host error reported verbatim, return",
            "effect": "no-op",
            "evidence": "crates/ui/src/connection.rs:258-272 — precedes the failures push and is unchanged"
          }
        ],
        "forbidden": [
          "reporting more than one terminal state per connection attempt",
          "shutting down or dropping the parked manager on the early-stop path (today's exhausted-list path does not, and the change must not alter routing/TUN teardown semantics)",
          "the early-stop Error text containing a per-node list, a node name, or U+001B",
          "starting a third candidate after two identical failures"
        ],
        "seeding": [
          "A candidate failure with a chosen message is seeded through the config-check path: the stub script answers `[ \"$1\" = check ]` by printing the chosen text to stderr and exiting 1. ProcessManager::check_config takes the last non-empty stderr line and returns ProcessError::ConfigCheck(reason) (crates/process/src/manager.rs:667-713), which is not host-level (manager.rs:2028 table), so spawn_with pushes it at connection.rs:273. This costs no crash budget — the 3-crash path is ~6s per candidate (CRASH_RESTART_DELAY 2s, MAX_CRASHES 3, manager.rs:36-37, 846).",
          "Launch count is observed by a marker file the stub appends to in its `check` branch. The path must exist before stub() is called, so the test creates its own `let launches = tempfile::tempdir().unwrap();` and formats the script with `launches.path().join(\"launched\").display()`; stub() then writes that script verbatim (crates/ui/src/connection.rs:521-533 — stub takes the script as &str and only prefixes `#!/bin/sh`). Count = lines in the marker file. The `version` branch must not append, so the count is candidates attempted, not process invocations. This mechanism does not exist at HEAD; the test creates it — no production change is needed for it.",
          "Corroborating observable, already used at HEAD: the generated config on disk is the last candidate written (connection.rs:186-193; asserted the same way in host_level_failure_stops_the_candidate_loop, connection.rs:921-927). After an early stop it must contain candidate 2's address and not candidate 3's.",
          "Per-candidate message variation is keyed off the config: `grep -q <address> \"$3\"` inside the stub, the pattern already used at connection.rs:663 and :695."
        ],
        "budgets": [
          "2.1: exactly 2 marker lines out of 3 candidates",
          "2.2: exactly 3 marker lines out of 3 candidates",
          "exactly 1 terminal state per test (drain() already asserts none follows, connection.rs:717-740)",
          "test wall clock: both tests take the config-check path, so no candidate spends the ~6s crash budget; RECV_TIMEOUT 20s per message is untouched"
        ]
      },
      "codeTasks": [
        "Label the candidate loop 'candidates and add `if repeats_previous(&failures) { break 'candidates; }` at the three sites described in the transitions above; leave every Stop check, the host-level branch, the parked handling and the forwarder halts untouched",
        "Replace the post-loop report with the branch: repeats_previous -> format!(\"Connection failed on consecutive nodes with the same error (not node-specific): {reason}\") using failures.last().reason; else summarize_failures(&failures)",
        "Test repeated_failure_stops_after_two_candidates (#[tokio::test(flavor = \"multi_thread\")]): 3 candidates 203.0.113.1/.2/.3, singbox_settings(), stub whose check branch appends \"$3\" to the marker file and prints `printf '\\033[31mFATAL\\033[0m[000%d] start service: post-start inbound/tun[tun-in]: starting TUN interface: set rules: add rule 0/9: address family not supported by protocol\\n' >&2` with a different bracketed counter per candidate (selected via grep on the config), exit 1. Assert: marker has 2 lines; drain() terminal is Error; msg starts with \"Connection failed on consecutive nodes with the same error (not node-specific): \"; msg contains \"address family not supported by protocol\"; !msg.contains('\\u{1b}'); !msg.contains(\"All candidates failed\"); msg does not contain \"203.0.113.\"; generated sing-box.json contains 203.0.113.2 and not 203.0.113.3",
        "Test differing_failures_try_every_candidate (#[tokio::test(flavor = \"multi_thread\")]): same 3 candidates, stub prints a distinct unrelated-host message per candidate (e.g. \"dial tcp 198.51.100.7:443: connection refused\" / .8 / .9, selected via grep on the config) and appends to the marker. Assert: marker has 3 lines; terminal Error starts with \"All candidates failed: \"; contains all three candidate labels; !msg.contains('\\u{1b}')",
        "Run 3.1 as the chunk floor",
        "3.2 is a live manual check, not automated and not a chunk gate: the plan records it as a hand-off note for the user after the floor is green. The original reproducer (the sing-box IPv6 TUN FATAL) no longer occurs at HEAD — fix-singbox-tun-without-ipv6 is archived as of d9aa189 — so reproduction needs a deliberately invalid TUN setting that fails at startup; the expected observation is two `session` records in backend.log for the attempt and the not-node-specific toast"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "- **THEN** the app SHALL NOT receive `Stopped` for the connection, SHALL keep its connection handle, and SHALL keep the TUN recovery marker",
      "tests": [
        "connection::tests::failover_does_not_report_stop"
      ]
    },
    {
      "shall": "- **THEN** Disconnect SHALL stop that backend and report `Stopped`",
      "tests": [
        "connection::tests::disconnect_after_failover"
      ]
    },
    {
      "shall": "- **THEN** the app SHALL receive exactly one terminal `Error` summarizing the failures",
      "tests": [
        "connection::tests::last_candidate_failure_reports_one_error"
      ]
    },
    {
      "shall": "- **THEN** no third candidate SHALL be started, and the app SHALL receive exactly one terminal `Error` that names the shared reason once, says it is not node-specific, and contains no color escape sequences",
      "tests": [
        "connection::tests::repeated_failure_stops_after_two_candidates"
      ]
    },
    {
      "shall": "- **THEN** the connection SHALL fail over to the third candidate",
      "tests": [
        "connection::tests::differing_failures_try_every_candidate"
      ]
    },
    {
      "shall": "## MODIFIED Requirements\n\n### Requirement: Connection terminal state has a single source\nThe system SHALL report a connection's terminal state (`Stopped` or `Error`) only from the component supervising the whole connection attempt, never from an individual candidate's backend.",
      "tests": [
        "connection::tests::last_candidate_failure_reports_one_error"
      ]
    },
    {
      "shall": "A candidate given up during failover SHALL NOT be reported as the connection stopping.",
      "tests": [
        "connection::tests::last_candidate_failure_reports_one_error"
      ]
    },
    {
      "shall": "When two consecutive candidates of the same connection attempt fail with the same reason — compared after removing terminal color codes, timestamps and elapsed-time counters, and each candidate's own name, address, and port — the system SHALL stop failing over and report a single `Error` stating that the same error repeated on consecutive nodes and is not node-specific, followed by that reason.",
      "tests": [
        "connection::tests::repeated_failure_stops_after_two_candidates"
      ]
    },
    {
      "shall": "Failure text reported to the user SHALL NOT contain terminal color escape sequences.",
      "tests": [
        "connection::tests::summarize_failures_strips_ansi"
      ]
    }
  ],
  "testHarness": [
    "stub — crates/ui/src/connection.rs:521 `fn stub(script: &str) -> Stub` — tempdir + AppPaths::for_profile_in(AppProfile::Test, ..) + a `#!/bin/sh` script at <tmp>/backend, 0o755. Per-candidate behavior is driven by grepping the generated config: `grep -q 203.0.113.1 \"$3\" && exit 1` (:663), and the `check` subcommand is branched separately: `[ \"$1\" = check ] && { grep -q 203.0.113.3 \"$3\" && exit 1; exit 0; }` (:695). Invocation shapes: check = `check -c <config>` so `$3` is the config (crates/process/src/manager.rs:681), run = `run -c <config>` (manager.rs:724).",
    "request / connect / connect_with — crates/ui/src/connection.rs:562 `fn request(`, :554 `fn connect(`, :584 `fn connect_with(` — builds ConnectionRequest (GENERATION = 7, host_has_ipv6 true) and calls spawn_with with a manager hook.",
    "next_state — crates/ui/src/connection.rs:601 `async fn next_state(` — filters AppMsg::ProcessStateConnection, asserts generation, RECV_TIMEOUT = 20s (:513).",
    "assert_nothing_after_terminal — crates/ui/src/connection.rs:618 — proves the task returned after the terminal state.",
    "drain — crates/ui/src/connection.rs:717 `async fn drain(` — returns (terminal state, all log lines) once the channel closes; the natural harness for the two new stub tests.",
    "candidate / node / singbox_settings — crates/ui/src/connection.rs:542, :1037, :595 — Shadowsocks node on port 8388, remark None so candidate_label == address.",
    "last_candidate_failure_reports_one_error — crates/ui/src/connection.rs:693 — 2 candidates: 203.0.113.1 passes `check` then crashes on every run (crash budget exhausted → `203.0.113.1: 3 crashes`), 203.0.113.3 fails `check` (→ `203.0.113.3: config rejected`). Asserts msg starts_with \"All candidates failed\", contains both per-node fragments, and assert_nothing_after_terminal. This is the mixed-failure regression guard for task 1.2.",
    "Launch counting: there is NO launch-count mechanism in the test module today. The nearest existing proxy is reading the generated config off disk and asserting which candidate's address it holds — crates/ui/src/connection.rs:919 `// Exactly one start attempt: the config on disk is still candidate` (:921-927). Note that proves how far config generation got, not how many launches happened. Two workable counters: (a) count `session backend=... node=<label>` records in <logs_dir>/backend.log (written once per launch at crates/process/src/manager.rs:522/:558, and `node=` carries candidate_label at :567-571) — but respawns after a crash write their own session record, so count distinct `node=` values, not records; (b) have the stub append a line to `$(dirname \"$3\")/launches` on the `run` path — the spawned command inherits the test process cwd (no current_dir is set, manager.rs:722-735), so an absolute/derived path is required, never a bare relative file."
  ],
  "targetedTests": [
    "failure_key_ignores_ansi_and_elapsed_counter",
    "failure_key_ignores_leading_timestamp",
    "failure_key_masks_candidate_identity",
    "failure_key_keeps_unrelated_hosts_distinct",
    "failure_key_keeps_midstring_timestamp",
    "summarize_failures_strips_ansi",
    "repeats_previous_needs_two_equal_keys",
    "repeated_failure_stops_after_two_candidates",
    "differing_failures_try_every_candidate",
    "last_candidate_failure_reports_one_error"
  ],
  "floor": "timeout 10m cargo test --workspace -- --test-threads=4 && timeout 10m cargo clippy -p v2ray-rs-ui --all-targets -- -D warnings",
  "manual": [
    "3.2 live reproduction of a shared startup failure — manual, cannot run in an automated run; note that a crash-budget failure writes one session record per launch including respawns, so the check is \"two distinct node= values\", not \"two session records\""
  ],
  "risks": [
    "design.md Context claims verified at HEAD d9aa189: three failure push sites (connection.rs:190, :273, :332), the 'All candidates failed: <first 3>; ...' summary (connection.rs:482-499), and the crash give-up text '3 crashes within 60s: process exited with code 1: <last output line>' (manager.rs:36-37, 801-809, 829-838, last_output_line at :907-918). No false claim found.",
    "The backend prefix samples in design.md (sing-box '\\u001b[31mFATAL\\u001b[0m[0000] ...', xray '2026/09/14 10:37:35.309646 [Error] ...') describe external binaries and cannot be verified from the repo; they are treated as the shape the normalization targets, and a miss only costs the old full-failover behavior.",
    "failure_key and repeats_previous are new symbols: a search for them across crates returned no hits, as expected for a change that introduces them. summarize_failures exists at exactly one call site (connection.rs:349) and one definition (:482).",
    "Over-normalization: a short or generic remark used as the candidate label could mask unrelated substrings and make two genuinely different failures compare equal. Mitigation: label and address are only substituted at length >= 3, and the port only on non-digit boundaries.",
    "Under-normalization: a volatile token outside the four handled classes keeps keys different, which degrades to today's full failover (design.md Risks) — never worse than HEAD.",
    "Two genuinely broken nodes failing identically end the attempt after two candidates (design.md Risks, accepted): the user sees the shared reason and automatic reconnect re-plans.",
    "Applying the same predicate to the exhausted-list exit means a run whose last two candidates fail identically now reports the not-node-specific wording instead of the per-node list. This follows the spec requirement text and is intentional; it changes no existing test (last_candidate_failure_reports_one_error's two failures differ: '3 crashes' vs 'config rejected').",
    "The stub marker file lives in a second TempDir owned by the test; it must outlive the connection task, so the guard is bound to a named local (`let launches = ...`), never `_`."
  ],
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 3
  }
}
```
