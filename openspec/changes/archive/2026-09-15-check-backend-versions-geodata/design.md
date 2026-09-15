## Context

- `crates/process/src/manager.rs` `start_with_connection` runs the per-candidate preflight. The xray version gate sits inside the TUN block (`&& let Some(triple) = self.xray_version_triple().await`). An unreadable version skips it silently. `ProcessError::TunBackendTooOld { installed }` is the variant this change generalizes to `BackendTooOld { backend, installed, required }`. It is classified host-level by `is_host_level()` (from `gate-tun-capability-preflight`), and the UI candidate loop (`if e.is_host_level() {` in `crates/ui/src/connection.rs`) ends the attempt on it.
- sing-box has no version gate. The generator avoids fields removed in 1.13.0. The verified runs are 1.13.14, 1.13.21 and 1.14.0 (`sing-box version 1.14.0` followed by an `Environment: go…` line). No older minimum is recorded, so 1.13.0 is used.
- The version probe runs `<binary> version` under `CONFIG_CHECK_TIMEOUT` (10 s) without `kill_on_drop` and without the ETXTBSY retry that `check_config` uses. The existing xray panic advisory goes to the log buffer and the event stream but not to `backend.log` (`write_stream_line`).
- Geodata: xray/v2ray rules emit `geoip:`/`geosite:` references (`crates/core/src/config/v2ray.rs`), loaded from `AppPaths::geodata_dir()` via `XRAY_LOCATION_ASSET`. `GeodataManager` resolves `geoip.dat`/`geosite.dat` in the same directory. sing-box falls back to remote rule-sets for uncached tags, so it is not gated.
- `resolve_effective_config` (`crates/core/src/models/imported_profile.rs`) applies an imported profile only to its own subscription's nodes. `singbox_rule_set_tags` (`crates/ui/src/geodata_service.rs`) walks every subscription's active profile.
- `crates/ui/src/app.rs`: every terminal `Error` becomes a toast. The action-toast pattern is the "Grant TUN privileges" button (`error_toast_action`). The geodata download exists only as inline closure code behind Preferences' "Update Now" (`crates/ui/src/preferences/network.rs`).

## Goals / Non-Goals

**Goals:**
- One version probe for both backends, applied before spawn. A backend too old for the generated config fails with a message naming the installed and required versions.
- Missing geodata that the connection's rules actually reference fails the connect before spawn, with a one-click fix.

**Non-Goals:**
- Gating sing-box on cached `.srs` files (the remote rule-set fallback covers them).
- Detecting corrupt, as opposed to missing, geodata. The backend's config check still fails as today.
- Capability/helper gates (`gate-tun-capability-preflight`) and recovery surfacing (`surface-tun-recovery-failures`).

## Decisions

- **sing-box minimum 1.13.0 for every sing-box start, checked before the TUN gates.** The gate goes right after the `ConfigMissing` check and before `if self.tun.is_some() {`. Granting privileges cannot fix a too-old binary, and xray already checks its version first. It applies without TUN too, because the generated config is the same. The rejected alternative, a TUN-only gate, would split identical configs arbitrarily. The xray gate and its unreadable-version warning stay TUN-only.
- **`BackendTooOld { backend: BackendType, installed: String, required: String }` replaces `TunBackendTooOld { installed }`.** Display: `installed {backend} {installed} is too old; {backend} {required} or newer is required`. `required` comes from `format_triple`, and `BackendType` displays as `sing-box`/`xray`. `XRAY_TUN_MIN_VERSION_STR` is deleted. `is_host_level()` stays true for the variant.
- **One probe, `version_triple`, spawned through `spawn_with_etxtbsy_retry` with `kill_on_drop(true)`.** The retry keeps parallel stub tests from misreading a busy text file as an unreadable version. `kill_on_drop` stops a stub that ignores `version` from leaking a child.
- **An unreadable version warns.** `warning: could not read <backend> version; minimum-version check skipped` goes to the log buffer, the event stream and `backend.log` through one helper that the panic advisory also moves onto. The start then continues, and the config check still runs.
- **`missing_geodata` is a private pure function in `app.rs`.** Signature: `missing_geodata(backend: BackendType, rules: &[RoutingRule], subscriptions: &[Subscription], dns: &DnsConfig, geoip_exists: bool, geosite_exists: bool) -> bool`. Geo references count only when they come from:
  - enabled global rules;
  - the global DNS rules, when `dns.enabled && dns.use_custom_rules`;
  - for each given subscription with `use_imported_profile` and a profile, its enabled rules and its DNS rules under the same condition.
  It returns false for sing-box.
- **The caller scopes subscriptions to the connection.** `start_connection` passes only subscriptions referenced by a candidate's `ConnectionNodeRef::Subscription`. That way a manual-node connect is not blocked by an unrelated provider's profile ("rules used for the connection").
- **Toast before any connection state.** When the predicate is true, `start_connection` shows `GeoIP/GeoSite rules need geodata that has not been downloaded` with a `Download geodata` button, before the connection generation is bumped. No task is spawned and the state does not change. The button sends a new `AppMsg::DownloadGeodata`. `ToastAction` is not extended, because it serves async errors tied to a connection generation.
- **Shared download.** The "Update Now" body moves into `geodata_service::update_geodata(paths, backend) -> Result<(), String>`, which Preferences and the toast action both call off the GTK thread. The outcome is toasted, and there is no auto-connect.

## Risks / Trade-offs

- [sing-box 1.12.x users whose configs happen to load] → blocked with a clear message. 1.12 is unverified against the current generator, and the constant is a one-line change.
- [Existing sing-box test stubs do not answer `version`] → each would burn the 10 s probe, against the UI tests' 20 s receive timeout. A crash-counting stub would also count the probe as a run. Every sing-box stub gains a `version` responder, and `live_connect_writes_backend_diagnostics` moves from 1.11.0 to 1.13.0.
- [Global rules/DNS are checked even when every candidate uses a profile that replaces them] → accepted over-approximation, cleared by one download. Per-candidate effective-config evaluation would change the predicate's signature.
- [Geodata present but corrupt] → not detected. The backend's config check still fails as today.
- [Repeated clicks start concurrent downloads] → both write through atomic persist, so no guard is added.
- [`warn-v2ray-tun-unsupported` modifies the same tun-mode requirement ("availability per backend")] → land the changes sequentially. Whichever archives second rebases its delta on the updated main spec.
- [`make clippy` (`-D warnings`) is red on the base commit because of a `private_interfaces` error in `crates/process/src/privilege.rs`] → the floor cannot pass until that is fixed. The fix lands before or with this change.

## Implementation plan

Tier **standard**, mode **existing-service-strict**, lenses **spec + quality** (standard tier; no auth, SQL, secrets or hot path is touched beyond the probe spawn already reviewed in the preceding change). Plan review: zarchitect, round 2 **pass** (round 1 raised one blocker, the subscription scoping, now carried in the `geodata-toast` contract).

Rust has no test-writer stage. Every seam carries `NO-RED-WAIVER: rust stack, tests authored by the coder as the first code task; NO-TESTER-WAIVER: verification = chunk cargo test + floor.` The coder writes the tests first. The chunk's other tests are pre-sealed, and only test files named in its sites may change.

Dispatch shape: `version-gate` and `geodata-predicate` run in parallel (disjoint files). `geodata-toast` follows `geodata-predicate` in the integration worktree.

### version-gate (task 1.1): parallel, shard `version`, rust-coder
- Sites: `crates/process/src/manager.rs`:
  - `async fn xray_version_triple(&self) -> Option<(u32, u32, u32)> {`
  - `const XRAY_TUN_MIN_VERSION_STR: &str = "26.1.13";`
  - `TunBackendTooOld { installed: String },`
  - `| ProcessError::TunBackendTooOld { .. }`
  - `&& let Some(triple) = self.xray_version_triple().await`
  - `if !self.config_path.exists() {` (sing-box gate follows it)
  - `let line = LogLine::stderr(format!(` (panic advisory)
  - `fn write_stream_line(`
  - the tests `tun_start_fails_fast_on_pre_tun_xray_version` and `is_host_level_classifies_every_variant`
  - the sing-box stubs
- Sites: `crates/ui/src/connection.rs`:
  - `grant_fixable_is_exactly_the_capability_pair`
  - `echo "sing-box 1.11.0"`
  - the sing-box stubs `grep -q 203.0.113.1 "$3" && exit 1; exec sleep 30`
- New tests:
  - `singbox_start_fails_before_check_on_old_version`
  - `singbox_start_proceeds_on_minimum_version`
  - `singbox_start_warns_on_unreadable_version`
  - `xray_tun_start_warns_on_unreadable_version`
  - `xray_start_without_tun_skips_version_probe`
- Migrated/extended tests: `tun_start_fails_fast_on_pre_tun_xray_version`, `is_host_level_classifies_every_variant`, `parse_semver_triple_from_xray_version_output`, `grant_fixable_is_exactly_the_capability_pair`, `live_connect_writes_backend_diagnostics`.
- Stub audit:
  - `manager.rs`: `config_check_failure_prevents_spawn`, `config_check_success_starts_backend`, `respawn_skips_preflight`, `stop_from_starting_without_child_reaches_stopped`, `singbox_tun_start_skips_helper_gates`, and every `.with_backend(BackendType::SingBox)`.
  - `connection.rs`: `failover_reports_no_stopped_and_stop_reports_one`, `last_candidate_failure_reports_one_error`, and every `singbox_settings()` caller.
- Budgets: 1 gate probe exec per preflight (the `backend_version` diagnostics probe with a log file is separate). The no-probe test builds the manager without a log file.
- Verify: `timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-process -p v2ray-rs-ui --all-targets && cargo fmt --check`

### geodata-predicate (task 2.1): parallel, integration worktree, rust-coder
- Sites (read-only references):
  - `pub(crate) fn singbox_rule_set_tags(` (`geodata_service.rs`)
  - `pub fn enabled_rules(&self)` (`routing.rs`)
  - `&& sub.use_imported_profile` (`imported_profile.rs`)
  - `pub enum DnsRuleMatch {` (`dns.rs`)
  - `self.geodata_dir.join("geosite.dat")` (`geodata.rs`)
- New function: `missing_geodata` in `crates/ui/src/app.rs` next to `fn error_toast_action`.
- Tests (`app::tests`):
  - `missing_geodata_xray_geosite_rule_without_file`
  - `missing_geodata_singbox_is_never_gated`
  - `missing_geodata_without_geo_rules`
  - `missing_geodata_imported_profile_geo_rule`
  - `missing_geodata_ignores_disabled_and_inactive_rules`
  - `missing_geodata_custom_dns_geosite_rule`
  - `missing_geodata_both_files_present`
  - `missing_geodata_one_file_missing`
- Verify: `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 missing_geodata && cargo clippy -p v2ray-rs-ui --all-targets && cargo fmt --check`

### geodata-toast (tasks 2.2, 3.1): serial after geodata-predicate (sharedPkg `crates/ui`), rust-coder
- Sites:
  - `self.connection_generation = self.connection_generation.wrapping_add(1);` (the check goes before it, in `start_connection`)
  - `.button_label("Grant TUN privileges")` (the toast pattern)
  - `AppMsg::ShowToast(message) => {`
  - the "Update Now" closure `download_geodata(&geodata_manager)` in `preferences/network.rs`
- Contract:
  - Blocked → toast + button; no generation bump, no `apply_state(Starting)`, no process handle.
  - Download → `spawn_blocking(update_geodata)` → outcome toast; no auto-connect.
  - Forbidden: subscriptions not referenced by any candidate reaching `missing_geodata`; a new `ToastAction` variant; a download on the GTK thread.
- Tests: none automatable beyond the predicate (GTK-bound). Manual check in the dev profile with `geosite.dat` removed: Connect → toast; Download geodata → success toast; Connect proceeds. Preferences "Update Now" behaves as before.
- Verify: `timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-process -p v2ray-rs-ui --all-targets && cargo fmt --check`

### Floor
`make test && make fmt && make clippy`. The Makefile applies `timeout 5m` and `--test-threads=4`, and `fmt` is `cargo fmt -- --check`.

### Rules for an executor without the orchestration tooling
- Work in a worktree off the base commit and assert `git -C <worktree> rev-parse --show-toplevel` before any edit.
- Once a contract test is written, it is read-only for later chunks. Change it only through an explicit amendment with one line of why.
- Commits follow Conventional Commits: subject ≤72 chars, imperative, lowercase; body lines ≤100. No `openspec/` paths in implementation commits.
- Every test run is resource-limited, as in the commands above.

## Plan appendix

```json
{
  "v": 2,
  "change": "check-backend-versions-geodata",
  "baseSha": "80fe2605962898597e9353487a7844b936557f3b",
  "generatedAt": "2026-09-15T17:15:15.319Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "estimateHours": 1.5,
  "chunks": [
    {
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "pkgs": [],
      "id": "version-gate",
      "taskIds": [
        "1.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "shard": "version",
      "seam": "a-backend-version-gate",
      "pkgDirs": [
        "crates/process/src",
        "crates/ui/src"
      ],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::xray_version_triple",
          "anchor": "async fn xray_version_triple(&self) -> Option<(u32, u32, u32)> {",
          "change": "Generalize into version_triple(&self) for any backend: spawn via crate::spawn::spawn_with_etxtbsy_retry with stdin null, stdout piped, kill_on_drop(true); wait_with_output under CONFIG_CHECK_TIMEOUT (10s); parse stdout via parse_semver_triple. sing-box prints `sing-box version 1.14.0` then `Environment: go…` (verified on /usr/bin/sing-box)."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "parse_semver_triple / format_triple",
          "anchor": "fn parse_semver_triple(text: &str) -> Option<(u32, u32, u32)> {",
          "change": "Reuse. It scans whitespace tokens for the first X.Y.Z, so `sing-box version 1.13.0` should parse. That output format was not checked against a real binary. Existing test: parse_semver_triple_from_xray_version_output."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "XRAY_TUN_MIN_VERSION consts",
          "anchor": "const XRAY_TUN_MIN_VERSION_STR: &str = \"26.1.13\";",
          "change": "Add SINGBOX_MIN_VERSION (1, 13, 0) next to XRAY_TUN_MIN_VERSION / XRAY_TUN_PANIC_FIX_VERSION. XRAY_TUN_MIN_VERSION_STR is only used in the TunBackendTooOld #[error] string, so it can go once `required` is a field (use format_triple)."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessError::TunBackendTooOld",
          "anchor": "TunBackendTooOld { installed: String },",
          "change": "Rename to BackendTooOld { backend, installed, required }. Current message: \"installed xray {installed} has no TUN inbound; TUN mode needs xray {XRAY_TUN_MIN_VERSION_STR} or newer\". Backend field type is not fixed yet: String or BackendType (Display exists, used as b.to_string() in write_session_record)."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessError::is_host_level",
          "anchor": "| ProcessError::TunBackendTooOld { .. }",
          "change": "Rename the arm to BackendTooOld { .. }; it stays true. That makes the sing-box gate end the candidate loop too (connection.rs `if e.is_host_level()`), as the process-lifecycle spec requires ('without trying further candidates')."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "start_with_connection xray gate",
          "anchor": "&& let Some(triple) = self.xray_version_triple().await",
          "change": "The xray gate is inside `if self.tun.is_some()`. It opens with a manual Starting→Error transition, not fail_with, so switch to `return self.fail_with(connection.as_ref(), error)`. Add an else branch for None that pushes the warning `warning: could not read xray version; minimum-version check skipped`. The panic advisory block (`if triple < XRAY_TUN_PANIC_FIX_VERSION`) stays as-is."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "start_with_connection sing-box gate site",
          "anchor": "if !self.config_path.exists() {",
          "change": "Place the sing-box gate right after the ConfigMissing check and before `if self.tun.is_some() {`; too old → `return self.fail_with(connection.as_ref(), error)`; unreadable → warning helper and proceed."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "panic advisory log push",
          "anchor": "let line = LogLine::stderr(format!(",
          "change": "Log-stream pattern: LogLine::stderr(..) → `if let Ok(mut buffer) = self.log_buffer.lock() { buffer.push(line.clone()); }` → `self.state.emit(ProcessEvent::LogLine(line))`. The advisory does NOT call write_stream_line, so it never reaches backend.log. Design wants the warning in backend.log too: call write_stream_line(&self.log_writer, \"stderr\"|<stream>, &text) first (the helper comment says file before buffer). Probably extract a small push_log_line helper shared with the advisory."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "write_stream_line",
          "anchor": "fn write_stream_line(writer: &Option<Arc<RotatingFileWriter>>, stream: &str, line: &str) {",
          "change": "Writes into backend.log. Stream names already in use: \"stdout\", \"session\", \"exit\", \"helper\"."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::check_config",
          "anchor": "BackendType::SingBox => &[\"check\", \"-c\"],",
          "change": "Reference only. check_config runs after the Starting transition. The sing-box gate must run before it: the test asserts no `check` invocation (see respawn_skips_preflight's checks-file stub)."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "tests::tun_start_fails_fast_on_pre_tun_xray_version",
          "anchor": "matches!(result, Err(ProcessError::TunBackendTooOld { ref installed }) if installed == \"25.12.8\"),",
          "change": "Migrate to BackendTooOld { installed, required, .. }."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "tests::is_host_level_classifies_every_variant",
          "anchor": "ProcessError::TunBackendTooOld {\n                    installed: \"25.12.8\".into(),",
          "change": "Migrate the case to BackendTooOld with all fields, still true."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "tests (new)",
          "anchor": "async fn config_check_failure_prevents_spawn() {",
          "change": "Add sing-box stub tests. (a) `version` → 'sing-box version 1.12.4' plus a checks file: expect Err(BackendTooOld), an empty checks file, and child none. (b) 1.13.0 → reaches check / Running. (c) empty version output → a log_buffer line containing 'minimum-version check skipped', start proceeds."
        },
        {
          "task": "1.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "existing sing-box stub tests without a version branch",
          "anchor": "let mut mgr = manager_for(&dir, \"[ \\\"$1\\\" = check ] && exit 0\\nexec sleep 30\\n\")",
          "change": "RISK: these now run the version probe. config_check_failure_prevents_spawn, config_check_success_starts_backend, respawn_skips_preflight and stop_from_starting_without_child_reaches_stopped answer `version` with `exec sleep 30` → a 10s timeout per start. singbox_tun_start_skips_helper_gates uses /bin/sh (empty stdout → warning only). Give each stub a version answer ≥1.13.0 or an immediate exit."
        },
        {
          "task": "1.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests::grant_fixable_is_exactly_the_capability_pair",
          "anchor": "ProcessError::TunBackendTooOld {\n                installed: \"25.3.5\".into(),",
          "change": "Migrate to BackendTooOld (must stay non-grant-fixable). grant_fixable itself (`fn grant_fixable(e: &ProcessError) -> bool {`) does not name the variant."
        },
        {
          "task": "1.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests::live_connect_writes_backend_diagnostics",
          "anchor": "echo \"sing-box 1.11.0\"",
          "change": "RISK: the stub reports 1.11.0 and would now be blocked by the gate. Bump to ≥1.13.0 and update the assertion `backend=sing-box version=1.11.0 node=203.0.113.1 tun=off`."
        },
        {
          "task": "1.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "sing-box stub tests without a version branch",
          "anchor": "grep -q 203.0.113.1 \"$3\" && exit 1; exec sleep 30",
          "change": "RISK: failover_reports_no_stopped_and_stop_reports_one and last_candidate_failure_reports_one_error answer `version` with exec sleep 30 → 10s per candidate against RECV_TIMEOUT 20s. log_lines_carry_connection_generation loops forever → 10s. no_marker_when_launched_without_tun (`exit 1`) → warning only. Add version answers."
        },
        {
          "task": "1.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "candidate loop host-level exit",
          "anchor": "if e.is_host_level() {",
          "change": "No code change. It already stops failover and reports e.to_string() for host-level errors, so the sing-box BackendTooOld rides it."
        }
      ],
      "coder": "rust-coder",
      "verify": "timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-process -p v2ray-rs-ui --all-targets && cargo fmt --check",
      "contract": {
        "states": [
          "VersionOk (proceed to check_config, no notice)",
          "PanicAdvisory (xray TUN 26.1.13..26.6.22: notice + proceed; existing)",
          "VersionUnreadable (warning notice + proceed)",
          "BackendTooOld (Err, state Error(display), no check_config, no child)",
          "NoProbe (gate not applicable, `version` never executed)"
        ],
        "transitions": [
          {
            "input": "sing-box, version stdout `sing-box version 1.12.4`, TUN off",
            "state": "BackendTooOld { backend: SingBox, installed: \"1.12.4\", required: \"1.13.0\" }; check_config not run; child None; state Error(msg contains 1.12.4 and 1.13.0)",
            "effect": "forced",
            "evidence": "process-lifecycle delta 'Scenario: Old sing-box blocked'"
          },
          {
            "input": "sing-box, version 1.12.4, TUN on",
            "state": "BackendTooOld (before mount/helper/getcap gates; no getcap invoked)",
            "effect": "forced",
            "evidence": "process-lifecycle delta 'Starting a sing-box connection with an older installed version SHALL fail before the backend is spawned'; placement decision"
          },
          {
            "input": "sing-box, version `sing-box version 1.13.0`",
            "state": "VersionOk -> check_config runs -> Running",
            "effect": "no-op",
            "evidence": "process-lifecycle delta 'Scenario: Supported sing-box starts'"
          },
          {
            "input": "sing-box, version 1.14.0 (or any >= 1.13.0)",
            "state": "VersionOk",
            "effect": "no-op",
            "evidence": "same scenario ('1.13.0 or newer')"
          },
          {
            "input": "sing-box, version stdout empty / unparseable / non-zero exit / spawn error / 10 s timeout",
            "state": "VersionUnreadable: log line `warning: could not read sing-box version; minimum-version check skipped`; check_config runs",
            "effect": "set",
            "evidence": "process-lifecycle delta 'Scenario: Unreadable sing-box version'"
          },
          {
            "input": "xray, TUN on, version `Xray 25.12.8 (...)`",
            "state": "BackendTooOld { backend: Xray, installed: \"25.12.8\", required: \"26.1.13\" }",
            "effect": "forced",
            "evidence": "tun-mode delta 'Scenario: Old xray blocks TUN start'; manager.rs `if triple < XRAY_TUN_MIN_VERSION {`"
          },
          {
            "input": "xray, TUN on, version 26.3.27",
            "state": "PanicAdvisory (existing `Xray-core #6364` line) then proceed to later gates",
            "effect": "set",
            "evidence": "manager.rs `if triple < XRAY_TUN_PANIC_FIX_VERSION {`; tun-mode 'Panic-affected xray warns but starts'"
          },
          {
            "input": "xray, TUN on, version >= 26.6.27",
            "state": "VersionOk",
            "effect": "no-op",
            "evidence": "tun-mode requirement text"
          },
          {
            "input": "xray, TUN on, version unreadable",
            "state": "VersionUnreadable: `warning: could not read xray version; minimum-version check skipped`; later TUN gates still run",
            "effect": "set",
            "evidence": "tun-mode delta 'Scenario: Unreadable xray version warns'"
          },
          {
            "input": "xray, TUN off, any version",
            "state": "NoProbe",
            "effect": "no-op",
            "evidence": "tun-mode requirement: xray minimum applies to TUN connections only"
          },
          {
            "input": "v2ray, any TUN flag",
            "state": "NoProbe",
            "effect": "no-op",
            "evidence": "no v2ray minimum in any delta"
          },
          {
            "input": "backend None (no with_backend)",
            "state": "NoProbe",
            "effect": "no-op",
            "evidence": "manager.rs `check_config` early return `let Some(backend) = self.backend else`"
          },
          {
            "input": "binary missing or config missing",
            "state": "existing BinaryNotFound / ConfigMissing; NoProbe",
            "effect": "no-op",
            "evidence": "manager.rs `if !self.binary_path.exists() {` precedes the gate"
          },
          {
            "input": "BackendTooOld.is_host_level()",
            "state": "true for both backends",
            "effect": "forced",
            "evidence": "manager.rs `pub fn is_host_level`; proposal Impact"
          },
          {
            "input": "ui candidate loop receives BackendTooOld on candidate 1 of N",
            "state": "loop stops, report Error(e.to_string()), no TunGrantRequired emitted",
            "effect": "forced",
            "evidence": "connection.rs `if e.is_host_level() {` and `fn grant_fixable`; spec 'end the connection attempt without trying further candidates'"
          }
        ],
        "forbidden": [
          "BackendTooOld with check_config having executed (stub `check` branch leaves a marker file; must be absent)",
          "BackendTooOld with mgr.child Some",
          "VersionUnreadable warning emitted for xray without TUN, for v2ray, or when version parsed",
          "BackendTooOld and the unreadable warning in the same start",
          "is_host_level() == false for any BackendTooOld",
          "grant_fixable(BackendTooOld) == true",
          "a `version` probe leaving a live child after timeout (kill_on_drop)"
        ],
        "seeding": [
          "All states: process-crate tests via `manager_for(&dir, script)` (manager.rs `fn manager_for`) + `.with_backend(BackendType::SingBox|Xray)`; the stub script answers `[ \"$1\" = version ] && { echo '<text>'; exit 0; }` first. Never set `cached_version` or fields directly.",
          "BackendTooOld sing-box: stub `[ \"$1\" = version ] && { echo 'sing-box version 1.12.4'; exit 0; }; [ \"$1\" = check ] && { touch <dir>/checked; exit 0; }; exec sleep 30`.",
          "VersionOk sing-box: same with `1.13.0`, then `mgr.start_with_connection(None).await.unwrap()`, assert Running, `mgr.stop().await`.",
          "VersionUnreadable sing-box: `[ \"$1\" = version ] && exit 0; [ \"$1\" = check ] && exit 0; exec sleep 30` -> Running; assert `mgr.log_buffer().lock().unwrap().last_n(10).iter().any(|l| l.content == \"warning: could not read sing-box version; minimum-version check skipped\")`.",
          "xray TUN states: copy the `tun_start_fails_fast_on_pre_tun_xray_version` fixture (TunRuntime with helper_path `missing-netctl`); after the version gate the start ends at TunHelperMissing or the mount gate, which is fine: assert on log_buffer / the returned variant only. Prefer `.with_host_probe(HostProbe { getcap: PathBuf::from(\"/bin/true\"), helper: <missing> })` to pin the mount gate off, as the existing preflight tests do.",
          "NoProbe xray non-TUN: stub version branch does `touch <dir>/probed`; assert file absent after start.",
          "is_host_level: literal construction in `is_host_level_classifies_every_variant` (pure enum value, legal)."
        ],
        "budgets": [
          "`version` probe bounded by CONFIG_CHECK_TIMEOUT = 10 s, killed on drop",
          "chunk test run: `timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4`; no stub may reach the 10 s probe timeout (every sing-box stub must answer `version` or exit immediately)",
          "1 gate probe exec per preflight; the backend_version diagnostics probe (write_session_record, only with a log file) is separate and unchanged",
          "NoProbe test seeds manager_for without with_log_file so a probed-marker proves the gate never ran"
        ],
        "names": [
          "ProcessError::BackendTooOld { backend: BackendType, installed: String, required: String } with #[error(\"installed {backend} {installed} is too old; {backend} {required} or newer is required\")]; required = format_triple(min); BackendType Display gives sing-box / xray",
          "const SINGBOX_MIN_VERSION: (u32, u32, u32) = (1, 13, 0); XRAY_TUN_MIN_VERSION kept; XRAY_TUN_MIN_VERSION_STR deleted",
          "warning line: format!(\"warning: could not read {backend} version; minimum-version check skipped\") — log buffer + event stream + backend.log through one helper the panic advisory also uses",
          "probe renamed version_triple; spawned through crate::spawn::spawn_with_etxtbsy_retry, stdin null, stdout piped, kill_on_drop(true), wait_with_output under CONFIG_CHECK_TIMEOUT (ETXTBSY retry prevents fixture flakes)"
        ],
        "placement": [
          "sing-box gate sits right after the ConfigMissing check and before `if self.tun.is_some() {` in start_with_connection, so it precedes mount/helper/getcap gates; too old → `return self.fail_with(connection.as_ref(), error)`",
          "xray gate and its unreadable warning stay inside the TUN block"
        ]
      },
      "codeTasks": [
        "T1 tests (manager.rs mod tests): `singbox_start_fails_before_check_on_old_version`, `singbox_start_proceeds_on_minimum_version`, `singbox_start_warns_on_unreadable_version`, `xray_tun_start_warns_on_unreadable_version`, `xray_start_without_tun_skips_version_probe`; extend `parse_semver_triple_from_xray_version_output` with `sing-box version 1.13.0\\n\\nEnvironment: go1.24.1 linux/amd64` -> Some((1, 13, 0)); add SINGBOX_MIN_VERSION comparisons next to `xray_tun_minimum_version_comparison`.",
        "T2 test migration (manager.rs): `tun_start_fails_fast_on_pre_tun_xray_version` matches `BackendTooOld { backend: BackendType::Xray, ref installed, ref required }` with installed == 25.12.8 and required == 26.1.13; `is_host_level_classifies_every_variant` gets BackendTooOld for Xray and SingBox, both true.",
        "T3 stub migration (manager.rs): every `.with_backend(BackendType::SingBox)` stub whose `version` path would sleep or count as a run gets a leading `[ \"$1\" = version ] && { echo 'sing-box version 1.13.0'; exit 0; }` — anchors: `config_check_failure_prevents_spawn`, `config_check_success_starts_backend`, the crash-count stub `echo run >> {runs}` used with SingBox (version exec would otherwise increment the run counter), the config-check-timeout stub `[ \"$1\" = check ] && exec sleep 30`, and the SingBox manager built at the `.with_backend(BackendType::SingBox);` after `manager_for` near the TUN sing-box tests.",
        "T4 test migration (ui connection.rs): `grant_fixable_is_exactly_the_capability_pair` uses `BackendTooOld { backend: BackendType::Xray, installed: \"25.3.5\".into(), required: \"26.1.13\".into() }`; `live_connect_writes_backend_diagnostics` stub `sing-box 1.11.0` -> `sing-box 1.13.0` and assertion `backend=sing-box version=1.13.0 node=203.0.113.1 tun=off`; every `singbox_settings()` stub (`fn stub(` callers: `failover_reports_no_stopped_and_stop_reports_one`, `last_candidate_failure_reports_one_error`, `no_marker_when_launched_without_tun`, others) gets the version responder, else each candidate burns the 10 s probe against RECV_TIMEOUT = 20 s. Note `no_marker_when_launched_without_tun` stub `exit 1` is unreadable -> warns and proceeds -> still fails at check: no change needed.",
        "T5 impl: SINGBOX_MIN_VERSION, BackendTooOld variant + is_host_level arm, delete XRAY_TUN_MIN_VERSION_STR, rename probe to version_triple with kill_on_drop, push_notice helper (panic advisory migrates onto it), sing-box gate before the TUN block, xray gate None arm warns.",
        "T6: grep the workspace for `TunBackendTooOld` and `xray_version_triple` -> zero hits (current hits: manager.rs x5, ui connection.rs x1).",
        "T3/T4 explicit stub audit — manager.rs: config_check_failure_prevents_spawn, config_check_success_starts_backend, respawn_skips_preflight (crash counter must not count `version`), stop_from_starting_without_child_reaches_stopped, the sing-box TUN test `singbox_tun_start_skips_helper_gates`; every `.with_backend(BackendType::SingBox)` in manager.rs tests. connection.rs: failover_reports_no_stopped_and_stop_reports_one, last_candidate_failure_reports_one_error, live_connect_writes_backend_diagnostics (1.11.0 → 1.13.0), every `singbox_settings()` caller; no_marker_when_launched_without_tun needs no change."
      ]
    },
    {
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "pkgs": [],
      "id": "geodata-predicate",
      "taskIds": [
        "2.1"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "shard": "",
      "seam": "b-geodata-predicate",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/ui/src/geodata_service.rs",
          "symbol": "singbox_rule_set_tags / rule_set_tag",
          "anchor": "pub(crate) fn singbox_rule_set_tags(",
          "change": "Closest precedent for missing_geodata. It walks rules.rules() filtered by enabled plus every sub with `use_imported_profile` && `Some(imported_profile)` → profile.rules filtered by enabled, matching RuleMatch::GeoIp/GeoSite. It lives in this sibling module with its own tests (geo_rule, vless_node fixtures). The task says app.rs; this module is the natural sibling."
        },
        {
          "task": "2.1",
          "file": "crates/core/src/models/routing.rs",
          "symbol": "RuleMatch / RoutingRuleSet::enabled_rules",
          "anchor": "pub fn enabled_rules(&self) -> impl Iterator<Item = &RoutingRule> {",
          "change": "Input. RuleMatch variants: `GeoIp { country_code: String }` and `GeoSite { category: String }`."
        },
        {
          "task": "2.1",
          "file": "crates/core/src/models/imported_profile.rs",
          "symbol": "ImportedProfile / resolve_effective_config",
          "anchor": "&& sub.use_imported_profile\n        && let Some(profile) = &sub.imported_profile",
          "change": "Reference. ImportedProfile { rules: Vec<RoutingRule>, dns: Option<DnsConfig>, .. }. Per candidate, profile rules (enabled) replace the global rules and profile.dns replaces settings.dns. `Subscription.use_imported_profile` defaults true (subscription.rs)."
        },
        {
          "task": "2.1",
          "file": "crates/core/src/models/dns.rs",
          "symbol": "DnsConfig.use_custom_rules / DnsRuleMatch::GeoSite",
          "anchor": "pub enum DnsRuleMatch {",
          "change": "Input. Only DnsRuleMatch::GeoSite { category } is geo; there is no GeoIp variant. The xray generator emits `geosite:{category}` from dns.rules only when `settings.dns.use_custom_rules` (v2ray.rs build_user_dns_servers). In non-custom mode, DNS domains come from routing GeoSite rules, which the rules check already covers."
        },
        {
          "task": "2.1",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "build_routing_rule",
          "anchor": "\"ip\": [format!(\"geoip:{}\", country_code.to_lowercase())],",
          "change": "Reference. The xray/v2ray generator emits geoip:<cc> for every GeoIp, including `private` (no special case, unlike sing-box's rule_set_tag), so GeoIp private also needs geoip.dat."
        },
        {
          "task": "2.1",
          "file": "crates/core/src/geodata.rs",
          "symbol": "GeodataManager::geoip_path / geosite_path",
          "anchor": "self.geodata_dir.join(\"geosite.dat\")",
          "change": "File names for the existence checks. AppPaths::geodata_dir() (persistence/mod.rs `pub fn geodata_dir(&self) -> PathBuf {`) = cache_dir/geodata. The caller computes geoip_exists/geosite_exists via GeodataManager::new(&paths).geoip_path().exists() or paths.geodata_dir().join(\"geoip.dat\")."
        }
      ],
      "coder": "rust-coder",
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 missing_geodata && cargo clippy -p v2ray-rs-ui --all-targets && cargo fmt --check",
      "contract": {
        "states": [
          "Missing (true: block connect)",
          "Satisfied (false: proceed)"
        ],
        "transitions": [
          {
            "input": "xray, enabled global GeoSite google, geosite_exists=false, geoip_exists=false",
            "state": "Missing",
            "effect": "set",
            "evidence": "geodata-management delta 'Scenario: Missing geodata blocks an xray connect'"
          },
          {
            "input": "v2ray, enabled global GeoIp ru, geoip_exists=false",
            "state": "Missing",
            "effect": "set",
            "evidence": "requirement 'backend is v2ray or xray'"
          },
          {
            "input": "xray, geo rule, geoip_exists=true, geosite_exists=false (or the reverse)",
            "state": "Missing",
            "effect": "set",
            "evidence": "requirement 'geoip.dat and geosite.dat exist'"
          },
          {
            "input": "xray, geo rule, both files exist",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "requirement"
          },
          {
            "input": "sing-box, geo rules, no files",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "'Scenario: sing-box is not gated'"
          },
          {
            "input": "xray, only Domain/IpCidr rules, no files",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "'Scenario: Rules without geo references'"
          },
          {
            "input": "xray, GeoSite rule with enabled=false, no files",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "requirement 'enabled routing rules'; v2ray generator receives enabled rules only (app.rs `rules.enabled_rules()`)"
          },
          {
            "input": "xray, subscription use_imported_profile=true with enabled GeoSite profile rule, no global rules, no files",
            "state": "Missing",
            "effect": "set",
            "evidence": "task 2.1 'imported profile geo rule -> true'; imported_profile.rs resolve_effective_config"
          },
          {
            "input": "xray, subscription use_imported_profile=false with GeoSite profile rule",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "resolve_effective_config `&& sub.use_imported_profile`"
          },
          {
            "input": "xray, active profile with GeoSite rule enabled=false",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "resolve_effective_config `.filter(|r| r.enabled)`"
          },
          {
            "input": "xray, dns.enabled=true, use_custom_rules=true, DnsRule GeoSite netflix, no files",
            "state": "Missing",
            "effect": "set",
            "evidence": "v2ray.rs `DnsRuleMatch::GeoSite { category } => format!(\"geosite:{category}\")`"
          },
          {
            "input": "xray, dns.enabled=true, use_custom_rules=false, DnsRule GeoSite",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "v2ray.rs `if settings.dns.use_custom_rules {`"
          },
          {
            "input": "xray, dns.enabled=false, use_custom_rules=true, DnsRule GeoSite",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "v2ray.rs `let mut servers = if settings.dns.enabled {`"
          },
          {
            "input": "xray, active profile dns Some(enabled, use_custom_rules, GeoSite rule), global dns without rules",
            "state": "Missing",
            "effect": "set",
            "evidence": "imported_profile.rs `effective.dns = dns.clone()`"
          }
        ],
        "forbidden": [
          "true for BackendType::SingBox",
          "true when both geoip_exists and geosite_exists",
          "true with no enabled geo reference in the evaluated rule sources",
          "predicate touching the filesystem (existence is passed in as bools)"
        ],
        "seeding": [
          "Rules: `RoutingRule { id: Uuid::new_v4(), match_condition: RuleMatch::GeoSite { category: \"google\".into() }, action: RuleAction::Proxy, enabled, group: None, via_node: None }` (imported_profile.rs `fn profile_rule`).",
          "Subscriptions with profile: follow geodata_service.rs tests (`sub.use_imported_profile = true; sub.imported_profile = Some(ImportedProfile { rules, dns: None, skipped: Vec::new(), imported_at: Utc::now() })`).",
          "DNS: `DnsConfig::default()` then set `enabled`, `use_custom_rules`, push `DnsRule { match_condition: DnsRuleMatch::GeoSite { category: \"netflix\".into() }, server_tag: <tag> }`.",
          "File existence: literal bools only."
        ],
        "budgets": [
          "O(rules + profile rules + dns rules), no I/O, no allocation required"
        ],
        "names": [
          "fn missing_geodata(backend: BackendType, rules: &[RoutingRule], subscriptions: &[Subscription], dns: &DnsConfig, geoip_exists: bool, geosite_exists: bool) -> bool — private fn in crates/ui/src/app.rs next to `fn error_toast_action`, tests in app::tests"
        ],
        "scope": [
          "walks every subscription it is given (use_imported_profile && imported_profile Some → its enabled rules + its dns when dns.enabled && use_custom_rules); the caller scopes subscriptions to the candidates",
          "global rules: enabled only; global dns rules only when dns.enabled && dns.use_custom_rules"
        ]
      },
      "codeTasks": [
        "T1 tests (app.rs mod tests): `missing_geodata_xray_geosite_rule_without_file` (true), `missing_geodata_singbox_is_never_gated` (false), `missing_geodata_without_geo_rules` (false), `missing_geodata_imported_profile_geo_rule` (true), plus `missing_geodata_ignores_disabled_and_inactive_rules`, `missing_geodata_custom_dns_geosite_rule` (true only when enabled && use_custom_rules), `missing_geodata_both_files_present` (false), `missing_geodata_one_file_missing` (true).",
        "T2 impl: the fn in app.rs below `fn error_toast_action`, `#[cfg_attr]`-free; used by seam c so no dead_code."
      ]
    },
    {
      "redTasks": [],
      "redTests": [],
      "redRun": "",
      "pkgs": [],
      "id": "geodata-toast",
      "taskIds": [
        "2.2",
        "3.1"
      ],
      "prev": "geodata-predicate",
      "sharedPkg": "crates/ui",
      "parallel": false,
      "shard": "",
      "seam": "c-connect-geodata-toast",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "sites": [
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::start_connection",
          "anchor": "self.connection_generation = self.connection_generation.wrapping_add(1);",
          "change": "Insert the missing_geodata check after rules load and before `self.connection_generation = self.connection_generation.wrapping_add(1);` / apply_state(Starting) / crate::connection::spawn. On true: show the action toast and return Err. Available here: self.settings.backend.backend_type, self.settings.dns, rules, subscriptions, self.paths. Callers: ConnectAuto (`let _ = self.start_connection(candidates, subscriptions, manual_nodes, &sender);`) and ConnectToNode (`.start_connection(vec![candidate], subscriptions, manual_nodes, &sender)` → session_target from .ok())."
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "grant toast (action toast pattern)",
          "anchor": ".button_label(\"Grant TUN privileges\")",
          "change": "Pattern to copy: adw::Toast::builder().title(..).button_label(..).build(); `let s = sender.input_sender().clone();` toast.connect_button_clicked(move |_| s.emit(AppMsg::..)); self.toast_overlay.add_toast(toast). Needs a new AppMsg variant (e.g. DownloadGeodata) near `TunGrantRequired(u64),`."
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "ToastAction / error_toast_action / apply_state",
          "anchor": "fn error_toast_action(generation: u64, grant_generation: Option<u64>) -> Option<ToastAction> {",
          "change": "Existing enum `ToastAction { GrantTun }`, used for Error states from the connection task. apply_state shows `Error: {msg}` unless the grant is armed. The geodata preflight fails before any ProcessState::Error, so it can toast directly from start_connection. Adding a ToastAction variant is optional (design call)."
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/preferences/network.rs",
          "symbol": "geodata_update_btn.connect_clicked (Update Now)",
          "anchor": "download_geodata(&geodata_manager)\n                                .map_err(|e| format!(\"Download failed: {}\", e))?;",
          "change": "Download to reuse. `glib::MainContext::default().spawn_local(async move { tokio::task::spawn_blocking(move || -> Result<String,String> { #[cfg(feature = \"geodata-fetch\")] {...} #[cfg(not(...))] Err(..) }).await; match result {...} })`. For non-sing-box: download_geodata(&GeodataManager::new(&paths)) then GeodataIndexManager::new(&paths).build_index(backend_type, &geoip_path, &geosite_path). This is inline closure code, not a named fn; the only shared fn is v2ray_rs_core::geodata::download_geodata (geodata.rs `pub fn download_geodata(manager: &GeodataManager)`). Extracting a shared helper would make this multi-file."
        },
        {
          "task": "2.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg::ShowToast / off-thread patterns in app",
          "anchor": "AppMsg::ShowToast(message) => {",
          "change": "Outcome toast route: emit AppMsg::ShowToast(String) from the async task via a cloned input_sender. Existing app.rs off-thread precedent: `tokio::spawn(async move { ... tokio::task::spawn_blocking(move || { recover_tun_session(...) }).await; s.emit(AppMsg::TunReleased); })`."
        }
      ],
      "coder": "rust-coder",
      "verify": "timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-process -p v2ray-rs-ui --all-targets && cargo fmt --check",
      "contract": {
        "states": [
          "Blocked (toast shown, no spawn)",
          "Proceed (existing connect path)",
          "Downloading (blocking task running)",
          "DownloadReported (outcome toast)"
        ],
        "transitions": [
          {
            "input": "Connect, xray, enabled GeoSite rule, geosite.dat absent",
            "state": "Blocked: toast title `GeoIP/GeoSite rules need geodata that has not been downloaded`, button `Download geodata`; start_connection returns Err; process_handle None; connection_generation unchanged; process_state unchanged",
            "effect": "forced",
            "evidence": "geodata-management delta 'Missing geodata blocks an xray connect'"
          },
          {
            "input": "Connect, missing_geodata false",
            "state": "Proceed",
            "effect": "no-op",
            "evidence": "'Rules without geo references', 'sing-box is not gated'"
          },
          {
            "input": "Connect with no binary configured",
            "state": "existing `No backend binary configured` toast first; geodata not checked",
            "effect": "no-op",
            "evidence": "app.rs `No backend binary configured — check Preferences` precedes"
          },
          {
            "input": "routing rules fail to load",
            "state": "existing `Failed to load routing rules` toast; geodata not checked",
            "effect": "no-op",
            "evidence": "app.rs `Failed to load routing rules`"
          },
          {
            "input": "click `Download geodata`",
            "state": "Downloading via spawn_blocking(update_geodata(paths, current backend))",
            "effect": "set",
            "evidence": "'Scenario: Download action fetches geodata'"
          },
          {
            "input": "download Ok",
            "state": "DownloadReported: toast `Geodata updated successfully`; no connect started",
            "effect": "set",
            "evidence": "same scenario 'report success'; decision no auto-connect"
          },
          {
            "input": "download Err / JoinError",
            "state": "DownloadReported: toast with `Download failed: ...` / `Index build failed: ...` / `Geodata download feature not enabled`",
            "effect": "set",
            "evidence": "same scenario 'or failure'; network.rs error strings"
          },
          {
            "input": "Preferences `Update Now`",
            "state": "unchanged behavior through update_geodata",
            "effect": "no-op",
            "evidence": "network.rs `.label(\"Update Now\")`"
          },
          {
            "input": "Connect to a manual node, xray, no global geo rules, an unrelated subscription with an active profile holding an enabled GeoSite rule, no files",
            "state": "Proceed",
            "effect": "no-op",
            "evidence": "crates/core/src/models/imported_profile.rs resolve_effective_config applies a profile only to its own subscription nodes; spec \"rules used for the connection\""
          }
        ],
        "forbidden": [
          "Blocked with apply_state(Starting), a generation bump, a new process_handle, or a ProcessState::Error toast",
          "download running on the GTK main thread",
          "a new ToastAction variant or a change to error_toast_action",
          "auto-connect after download",
          "subscriptions not referenced by any candidate reaching missing_geodata"
        ],
        "seeding": [
          "Manual only: run the dev profile (`cargo run -p v2ray-rs-ui -- --dev` or the repo's `make run-dev`), backend xray, add an enabled GeoSite rule, delete `geosite.dat` from the dev profile geodata dir (`AppPaths::new_dev()` -> `v2ray-rs-dev` data dir `geodata/`), click Connect -> toast; click Download geodata -> success toast; Connect proceeds.",
          "Predicate states: seam b seeding."
        ],
        "budgets": [
          "2 `Path::exists` calls on the GTK thread per connect attempt",
          "download bound by the existing reqwest blocking client timeouts in core geodata (unchanged)",
          "task 3.1: `timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-ui -- --test-threads=4` within 5 min"
        ],
        "callSite": [
          "in start_connection (anchor `fn start_connection(`), before `self.connection_generation = self.connection_generation.wrapping_add(1);`: subscriptions = those whose id matches a candidate `ConnectionNodeRef::Subscription { subscription_id, .. }` (ConnectionCandidate.node_ref, crates/core/src/resolve.rs); rules = &enabled_rules; dns = &self.settings.dns; backend = self.settings.backend.backend_type; existence via GeodataManager/paths.geodata_dir() joined with geoip.dat / geosite.dat"
        ]
      },
      "codeTasks": [
        "T1 tests: none automatable beyond seam b (GTK-bound); keep all existing ui tests green (`error_toast_action_arms_only_for_matching_generation` unchanged).",
        "T2 extract `geodata_service::update_geodata`; network.rs Update Now calls it.",
        "T3 `AppMsg::DownloadGeodata` variant + handler.",
        "T4 geodata check + actionable toast in `start_connection`.",
        "T5 manual dev-profile check (task 2.2).",
        "T6 task 3.1: run the chunk command, then the full floor."
      ]
    }
  ],
  "seams": [
    {
      "id": "a-backend-version-gate",
      "tasks": [
        "1.1"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests authored by the coder as the first code task; NO-TESTER-WAIVER: verification = chunk cargo test + floor. One version probe for sing-box (every start) and xray (TUN only), run in ProcessManager::start_with_connection after the BinaryNotFound/ConfigMissing checks and before every TUN mount/helper/capability gate and before check_config. ProcessError::TunBackendTooOld { installed } becomes ProcessError::BackendTooOld { backend, installed, required }; is_host_level() keeps it true; ui candidate loop stops failover on it unchanged. Unreadable version pushes one warning line to log_buffer + ProcessEvent::LogLine + backend.log and proceeds.",
      "contract": {
        "states": [
          "VersionOk (proceed to check_config, no notice)",
          "PanicAdvisory (xray TUN 26.1.13..26.6.22: notice + proceed; existing)",
          "VersionUnreadable (warning notice + proceed)",
          "BackendTooOld (Err, state Error(display), no check_config, no child)",
          "NoProbe (gate not applicable, `version` never executed)"
        ],
        "transitions": [
          {
            "input": "sing-box, version stdout `sing-box version 1.12.4`, TUN off",
            "state": "BackendTooOld { backend: SingBox, installed: \"1.12.4\", required: \"1.13.0\" }; check_config not run; child None; state Error(msg contains 1.12.4 and 1.13.0)",
            "effect": "forced",
            "evidence": "process-lifecycle delta 'Scenario: Old sing-box blocked'"
          },
          {
            "input": "sing-box, version 1.12.4, TUN on",
            "state": "BackendTooOld (before mount/helper/getcap gates; no getcap invoked)",
            "effect": "forced",
            "evidence": "process-lifecycle delta 'Starting a sing-box connection with an older installed version SHALL fail before the backend is spawned'; placement decision"
          },
          {
            "input": "sing-box, version `sing-box version 1.13.0`",
            "state": "VersionOk -> check_config runs -> Running",
            "effect": "no-op",
            "evidence": "process-lifecycle delta 'Scenario: Supported sing-box starts'"
          },
          {
            "input": "sing-box, version 1.14.0 (or any >= 1.13.0)",
            "state": "VersionOk",
            "effect": "no-op",
            "evidence": "same scenario ('1.13.0 or newer')"
          },
          {
            "input": "sing-box, version stdout empty / unparseable / non-zero exit / spawn error / 10 s timeout",
            "state": "VersionUnreadable: log line `warning: could not read sing-box version; minimum-version check skipped`; check_config runs",
            "effect": "set",
            "evidence": "process-lifecycle delta 'Scenario: Unreadable sing-box version'"
          },
          {
            "input": "xray, TUN on, version `Xray 25.12.8 (...)`",
            "state": "BackendTooOld { backend: Xray, installed: \"25.12.8\", required: \"26.1.13\" }",
            "effect": "forced",
            "evidence": "tun-mode delta 'Scenario: Old xray blocks TUN start'; manager.rs `if triple < XRAY_TUN_MIN_VERSION {`"
          },
          {
            "input": "xray, TUN on, version 26.3.27",
            "state": "PanicAdvisory (existing `Xray-core #6364` line) then proceed to later gates",
            "effect": "set",
            "evidence": "manager.rs `if triple < XRAY_TUN_PANIC_FIX_VERSION {`; tun-mode 'Panic-affected xray warns but starts'"
          },
          {
            "input": "xray, TUN on, version >= 26.6.27",
            "state": "VersionOk",
            "effect": "no-op",
            "evidence": "tun-mode requirement text"
          },
          {
            "input": "xray, TUN on, version unreadable",
            "state": "VersionUnreadable: `warning: could not read xray version; minimum-version check skipped`; later TUN gates still run",
            "effect": "set",
            "evidence": "tun-mode delta 'Scenario: Unreadable xray version warns'"
          },
          {
            "input": "xray, TUN off, any version",
            "state": "NoProbe",
            "effect": "no-op",
            "evidence": "tun-mode requirement: xray minimum applies to TUN connections only"
          },
          {
            "input": "v2ray, any TUN flag",
            "state": "NoProbe",
            "effect": "no-op",
            "evidence": "no v2ray minimum in any delta"
          },
          {
            "input": "backend None (no with_backend)",
            "state": "NoProbe",
            "effect": "no-op",
            "evidence": "manager.rs `check_config` early return `let Some(backend) = self.backend else`"
          },
          {
            "input": "binary missing or config missing",
            "state": "existing BinaryNotFound / ConfigMissing; NoProbe",
            "effect": "no-op",
            "evidence": "manager.rs `if !self.binary_path.exists() {` precedes the gate"
          },
          {
            "input": "BackendTooOld.is_host_level()",
            "state": "true for both backends",
            "effect": "forced",
            "evidence": "manager.rs `pub fn is_host_level`; proposal Impact"
          },
          {
            "input": "ui candidate loop receives BackendTooOld on candidate 1 of N",
            "state": "loop stops, report Error(e.to_string()), no TunGrantRequired emitted",
            "effect": "forced",
            "evidence": "connection.rs `if e.is_host_level() {` and `fn grant_fixable`; spec 'end the connection attempt without trying further candidates'"
          }
        ],
        "forbidden": [
          "BackendTooOld with check_config having executed (stub `check` branch leaves a marker file; must be absent)",
          "BackendTooOld with mgr.child Some",
          "VersionUnreadable warning emitted for xray without TUN, for v2ray, or when version parsed",
          "BackendTooOld and the unreadable warning in the same start",
          "is_host_level() == false for any BackendTooOld",
          "grant_fixable(BackendTooOld) == true",
          "a `version` probe leaving a live child after timeout (kill_on_drop)"
        ],
        "seeding": [
          "All states: process-crate tests via `manager_for(&dir, script)` (manager.rs `fn manager_for`) + `.with_backend(BackendType::SingBox|Xray)`; the stub script answers `[ \"$1\" = version ] && { echo '<text>'; exit 0; }` first. Never set `cached_version` or fields directly.",
          "BackendTooOld sing-box: stub `[ \"$1\" = version ] && { echo 'sing-box version 1.12.4'; exit 0; }; [ \"$1\" = check ] && { touch <dir>/checked; exit 0; }; exec sleep 30`.",
          "VersionOk sing-box: same with `1.13.0`, then `mgr.start_with_connection(None).await.unwrap()`, assert Running, `mgr.stop().await`.",
          "VersionUnreadable sing-box: `[ \"$1\" = version ] && exit 0; [ \"$1\" = check ] && exit 0; exec sleep 30` -> Running; assert `mgr.log_buffer().lock().unwrap().last_n(10).iter().any(|l| l.content == \"warning: could not read sing-box version; minimum-version check skipped\")`.",
          "xray TUN states: copy the `tun_start_fails_fast_on_pre_tun_xray_version` fixture (TunRuntime with helper_path `missing-netctl`); after the version gate the start ends at TunHelperMissing or the mount gate, which is fine: assert on log_buffer / the returned variant only. Prefer `.with_host_probe(HostProbe { getcap: PathBuf::from(\"/bin/true\"), helper: <missing> })` to pin the mount gate off, as the existing preflight tests do.",
          "NoProbe xray non-TUN: stub version branch does `touch <dir>/probed`; assert file absent after start.",
          "is_host_level: literal construction in `is_host_level_classifies_every_variant` (pure enum value, legal)."
        ],
        "budgets": [
          "`version` probe bounded by CONFIG_CHECK_TIMEOUT = 10 s, killed on drop",
          "chunk test run: `timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4`; no stub may reach the 10 s probe timeout (every sing-box stub must answer `version` or exit immediately)",
          "1 gate probe exec per preflight; the backend_version diagnostics probe (write_session_record, only with a log file) is separate and unchanged",
          "NoProbe test seeds manager_for without with_log_file so a probed-marker proves the gate never ran"
        ],
        "names": [
          "ProcessError::BackendTooOld { backend: BackendType, installed: String, required: String } with #[error(\"installed {backend} {installed} is too old; {backend} {required} or newer is required\")]; required = format_triple(min); BackendType Display gives sing-box / xray",
          "const SINGBOX_MIN_VERSION: (u32, u32, u32) = (1, 13, 0); XRAY_TUN_MIN_VERSION kept; XRAY_TUN_MIN_VERSION_STR deleted",
          "warning line: format!(\"warning: could not read {backend} version; minimum-version check skipped\") — log buffer + event stream + backend.log through one helper the panic advisory also uses",
          "probe renamed version_triple; spawned through crate::spawn::spawn_with_etxtbsy_retry, stdin null, stdout piped, kill_on_drop(true), wait_with_output under CONFIG_CHECK_TIMEOUT (ETXTBSY retry prevents fixture flakes)"
        ],
        "placement": [
          "sing-box gate sits right after the ConfigMissing check and before `if self.tun.is_some() {` in start_with_connection, so it precedes mount/helper/getcap gates; too old → `return self.fail_with(connection.as_ref(), error)`",
          "xray gate and its unreadable warning stay inside the TUN block"
        ]
      },
      "codeTasks": [
        "T1 tests (manager.rs mod tests): `singbox_start_fails_before_check_on_old_version`, `singbox_start_proceeds_on_minimum_version`, `singbox_start_warns_on_unreadable_version`, `xray_tun_start_warns_on_unreadable_version`, `xray_start_without_tun_skips_version_probe`; extend `parse_semver_triple_from_xray_version_output` with `sing-box version 1.13.0\\n\\nEnvironment: go1.24.1 linux/amd64` -> Some((1, 13, 0)); add SINGBOX_MIN_VERSION comparisons next to `xray_tun_minimum_version_comparison`.",
        "T2 test migration (manager.rs): `tun_start_fails_fast_on_pre_tun_xray_version` matches `BackendTooOld { backend: BackendType::Xray, ref installed, ref required }` with installed == 25.12.8 and required == 26.1.13; `is_host_level_classifies_every_variant` gets BackendTooOld for Xray and SingBox, both true.",
        "T3 stub migration (manager.rs): every `.with_backend(BackendType::SingBox)` stub whose `version` path would sleep or count as a run gets a leading `[ \"$1\" = version ] && { echo 'sing-box version 1.13.0'; exit 0; }` — anchors: `config_check_failure_prevents_spawn`, `config_check_success_starts_backend`, the crash-count stub `echo run >> {runs}` used with SingBox (version exec would otherwise increment the run counter), the config-check-timeout stub `[ \"$1\" = check ] && exec sleep 30`, and the SingBox manager built at the `.with_backend(BackendType::SingBox);` after `manager_for` near the TUN sing-box tests.",
        "T4 test migration (ui connection.rs): `grant_fixable_is_exactly_the_capability_pair` uses `BackendTooOld { backend: BackendType::Xray, installed: \"25.3.5\".into(), required: \"26.1.13\".into() }`; `live_connect_writes_backend_diagnostics` stub `sing-box 1.11.0` -> `sing-box 1.13.0` and assertion `backend=sing-box version=1.13.0 node=203.0.113.1 tun=off`; every `singbox_settings()` stub (`fn stub(` callers: `failover_reports_no_stopped_and_stop_reports_one`, `last_candidate_failure_reports_one_error`, `no_marker_when_launched_without_tun`, others) gets the version responder, else each candidate burns the 10 s probe against RECV_TIMEOUT = 20 s. Note `no_marker_when_launched_without_tun` stub `exit 1` is unreadable -> warns and proceeds -> still fails at check: no change needed.",
        "T5 impl: SINGBOX_MIN_VERSION, BackendTooOld variant + is_host_level arm, delete XRAY_TUN_MIN_VERSION_STR, rename probe to version_triple with kill_on_drop, push_notice helper (panic advisory migrates onto it), sing-box gate before the TUN block, xray gate None arm warns.",
        "T6: grep the workspace for `TunBackendTooOld` and `xray_version_triple` -> zero hits (current hits: manager.rs x5, ui connection.rs x1).",
        "T3/T4 explicit stub audit — manager.rs: config_check_failure_prevents_spawn, config_check_success_starts_backend, respawn_skips_preflight (crash counter must not count `version`), stop_from_starting_without_child_reaches_stopped, the sing-box TUN test `singbox_tun_start_skips_helper_gates`; every `.with_backend(BackendType::SingBox)` in manager.rs tests. connection.rs: failover_reports_no_stopped_and_stop_reports_one, last_candidate_failure_reports_one_error, live_connect_writes_backend_diagnostics (1.11.0 → 1.13.0), every `singbox_settings()` caller; no_marker_when_launched_without_tun needs no change."
      ]
    },
    {
      "id": "b-geodata-predicate",
      "tasks": [
        "2.1"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests authored by the coder as the first code task; NO-TESTER-WAIVER: verification = chunk cargo test + floor. Pure `missing_geodata` in crates/ui/src/app.rs (next to `fn error_toast_action`), mirroring the rule walk of `singbox_rule_set_tags` (geodata_service.rs) and the emission conditions of the v2ray/xray generator, so it blocks exactly when the generated config would reference `geoip:`/`geosite:` and a .dat is absent.",
      "contract": {
        "states": [
          "Missing (true: block connect)",
          "Satisfied (false: proceed)"
        ],
        "transitions": [
          {
            "input": "xray, enabled global GeoSite google, geosite_exists=false, geoip_exists=false",
            "state": "Missing",
            "effect": "set",
            "evidence": "geodata-management delta 'Scenario: Missing geodata blocks an xray connect'"
          },
          {
            "input": "v2ray, enabled global GeoIp ru, geoip_exists=false",
            "state": "Missing",
            "effect": "set",
            "evidence": "requirement 'backend is v2ray or xray'"
          },
          {
            "input": "xray, geo rule, geoip_exists=true, geosite_exists=false (or the reverse)",
            "state": "Missing",
            "effect": "set",
            "evidence": "requirement 'geoip.dat and geosite.dat exist'"
          },
          {
            "input": "xray, geo rule, both files exist",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "requirement"
          },
          {
            "input": "sing-box, geo rules, no files",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "'Scenario: sing-box is not gated'"
          },
          {
            "input": "xray, only Domain/IpCidr rules, no files",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "'Scenario: Rules without geo references'"
          },
          {
            "input": "xray, GeoSite rule with enabled=false, no files",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "requirement 'enabled routing rules'; v2ray generator receives enabled rules only (app.rs `rules.enabled_rules()`)"
          },
          {
            "input": "xray, subscription use_imported_profile=true with enabled GeoSite profile rule, no global rules, no files",
            "state": "Missing",
            "effect": "set",
            "evidence": "task 2.1 'imported profile geo rule -> true'; imported_profile.rs resolve_effective_config"
          },
          {
            "input": "xray, subscription use_imported_profile=false with GeoSite profile rule",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "resolve_effective_config `&& sub.use_imported_profile`"
          },
          {
            "input": "xray, active profile with GeoSite rule enabled=false",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "resolve_effective_config `.filter(|r| r.enabled)`"
          },
          {
            "input": "xray, dns.enabled=true, use_custom_rules=true, DnsRule GeoSite netflix, no files",
            "state": "Missing",
            "effect": "set",
            "evidence": "v2ray.rs `DnsRuleMatch::GeoSite { category } => format!(\"geosite:{category}\")`"
          },
          {
            "input": "xray, dns.enabled=true, use_custom_rules=false, DnsRule GeoSite",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "v2ray.rs `if settings.dns.use_custom_rules {`"
          },
          {
            "input": "xray, dns.enabled=false, use_custom_rules=true, DnsRule GeoSite",
            "state": "Satisfied",
            "effect": "no-op",
            "evidence": "v2ray.rs `let mut servers = if settings.dns.enabled {`"
          },
          {
            "input": "xray, active profile dns Some(enabled, use_custom_rules, GeoSite rule), global dns without rules",
            "state": "Missing",
            "effect": "set",
            "evidence": "imported_profile.rs `effective.dns = dns.clone()`"
          }
        ],
        "forbidden": [
          "true for BackendType::SingBox",
          "true when both geoip_exists and geosite_exists",
          "true with no enabled geo reference in the evaluated rule sources",
          "predicate touching the filesystem (existence is passed in as bools)"
        ],
        "seeding": [
          "Rules: `RoutingRule { id: Uuid::new_v4(), match_condition: RuleMatch::GeoSite { category: \"google\".into() }, action: RuleAction::Proxy, enabled, group: None, via_node: None }` (imported_profile.rs `fn profile_rule`).",
          "Subscriptions with profile: follow geodata_service.rs tests (`sub.use_imported_profile = true; sub.imported_profile = Some(ImportedProfile { rules, dns: None, skipped: Vec::new(), imported_at: Utc::now() })`).",
          "DNS: `DnsConfig::default()` then set `enabled`, `use_custom_rules`, push `DnsRule { match_condition: DnsRuleMatch::GeoSite { category: \"netflix\".into() }, server_tag: <tag> }`.",
          "File existence: literal bools only."
        ],
        "budgets": [
          "O(rules + profile rules + dns rules), no I/O, no allocation required"
        ],
        "names": [
          "fn missing_geodata(backend: BackendType, rules: &[RoutingRule], subscriptions: &[Subscription], dns: &DnsConfig, geoip_exists: bool, geosite_exists: bool) -> bool — private fn in crates/ui/src/app.rs next to `fn error_toast_action`, tests in app::tests"
        ],
        "scope": [
          "walks every subscription it is given (use_imported_profile && imported_profile Some → its enabled rules + its dns when dns.enabled && use_custom_rules); the caller scopes subscriptions to the candidates",
          "global rules: enabled only; global dns rules only when dns.enabled && dns.use_custom_rules"
        ]
      },
      "codeTasks": [
        "T1 tests (app.rs mod tests): `missing_geodata_xray_geosite_rule_without_file` (true), `missing_geodata_singbox_is_never_gated` (false), `missing_geodata_without_geo_rules` (false), `missing_geodata_imported_profile_geo_rule` (true), plus `missing_geodata_ignores_disabled_and_inactive_rules`, `missing_geodata_custom_dns_geosite_rule` (true only when enabled && use_custom_rules), `missing_geodata_both_files_present` (false), `missing_geodata_one_file_missing` (true).",
        "T2 impl: the fn in app.rs below `fn error_toast_action`, `#[cfg_attr]`-free; used by seam c so no dead_code."
      ]
    },
    {
      "id": "c-connect-geodata-toast",
      "tasks": [
        "2.2",
        "3.1"
      ],
      "summary": "NO-RED-WAIVER: rust stack, tests authored by the coder as the first code task; NO-TESTER-WAIVER: verification = chunk cargo test + floor. `App::start_connection` calls `missing_geodata` before any connection state changes; on true it shows an actionable toast and returns Err without spawning. The toast button emits a new `AppMsg::DownloadGeodata`, whose handler runs the Preferences 'Update Now' routine (extracted to geodata_service.rs) via spawn_local + spawn_blocking and toasts the outcome. GTK-bound: covered by the manual dev-profile check plus the seam-b unit tests; task 3.1 is the chunk verification.",
      "contract": {
        "states": [
          "Blocked (toast shown, no spawn)",
          "Proceed (existing connect path)",
          "Downloading (blocking task running)",
          "DownloadReported (outcome toast)"
        ],
        "transitions": [
          {
            "input": "Connect, xray, enabled GeoSite rule, geosite.dat absent",
            "state": "Blocked: toast title `GeoIP/GeoSite rules need geodata that has not been downloaded`, button `Download geodata`; start_connection returns Err; process_handle None; connection_generation unchanged; process_state unchanged",
            "effect": "forced",
            "evidence": "geodata-management delta 'Missing geodata blocks an xray connect'"
          },
          {
            "input": "Connect, missing_geodata false",
            "state": "Proceed",
            "effect": "no-op",
            "evidence": "'Rules without geo references', 'sing-box is not gated'"
          },
          {
            "input": "Connect with no binary configured",
            "state": "existing `No backend binary configured` toast first; geodata not checked",
            "effect": "no-op",
            "evidence": "app.rs `No backend binary configured — check Preferences` precedes"
          },
          {
            "input": "routing rules fail to load",
            "state": "existing `Failed to load routing rules` toast; geodata not checked",
            "effect": "no-op",
            "evidence": "app.rs `Failed to load routing rules`"
          },
          {
            "input": "click `Download geodata`",
            "state": "Downloading via spawn_blocking(update_geodata(paths, current backend))",
            "effect": "set",
            "evidence": "'Scenario: Download action fetches geodata'"
          },
          {
            "input": "download Ok",
            "state": "DownloadReported: toast `Geodata updated successfully`; no connect started",
            "effect": "set",
            "evidence": "same scenario 'report success'; decision no auto-connect"
          },
          {
            "input": "download Err / JoinError",
            "state": "DownloadReported: toast with `Download failed: ...` / `Index build failed: ...` / `Geodata download feature not enabled`",
            "effect": "set",
            "evidence": "same scenario 'or failure'; network.rs error strings"
          },
          {
            "input": "Preferences `Update Now`",
            "state": "unchanged behavior through update_geodata",
            "effect": "no-op",
            "evidence": "network.rs `.label(\"Update Now\")`"
          },
          {
            "input": "Connect to a manual node, xray, no global geo rules, an unrelated subscription with an active profile holding an enabled GeoSite rule, no files",
            "state": "Proceed",
            "effect": "no-op",
            "evidence": "crates/core/src/models/imported_profile.rs resolve_effective_config applies a profile only to its own subscription nodes; spec \"rules used for the connection\""
          }
        ],
        "forbidden": [
          "Blocked with apply_state(Starting), a generation bump, a new process_handle, or a ProcessState::Error toast",
          "download running on the GTK main thread",
          "a new ToastAction variant or a change to error_toast_action",
          "auto-connect after download",
          "subscriptions not referenced by any candidate reaching missing_geodata"
        ],
        "seeding": [
          "Manual only: run the dev profile (`cargo run -p v2ray-rs-ui -- --dev` or the repo's `make run-dev`), backend xray, add an enabled GeoSite rule, delete `geosite.dat` from the dev profile geodata dir (`AppPaths::new_dev()` -> `v2ray-rs-dev` data dir `geodata/`), click Connect -> toast; click Download geodata -> success toast; Connect proceeds.",
          "Predicate states: seam b seeding."
        ],
        "budgets": [
          "2 `Path::exists` calls on the GTK thread per connect attempt",
          "download bound by the existing reqwest blocking client timeouts in core geodata (unchanged)",
          "task 3.1: `timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-ui -- --test-threads=4` within 5 min"
        ],
        "callSite": [
          "in start_connection (anchor `fn start_connection(`), before `self.connection_generation = self.connection_generation.wrapping_add(1);`: subscriptions = those whose id matches a candidate `ConnectionNodeRef::Subscription { subscription_id, .. }` (ConnectionCandidate.node_ref, crates/core/src/resolve.rs); rules = &enabled_rules; dns = &self.settings.dns; backend = self.settings.backend.backend_type; existence via GeodataManager/paths.geodata_dir() joined with geoip.dat / geosite.dat"
        ]
      },
      "codeTasks": [
        "T1 tests: none automatable beyond seam b (GTK-bound); keep all existing ui tests green (`error_toast_action_arms_only_for_matching_generation` unchanged).",
        "T2 extract `geodata_service::update_geodata`; network.rs Update Now calls it.",
        "T3 `AppMsg::DownloadGeodata` variant + handler.",
        "T4 geodata check + actionable toast in `start_connection`.",
        "T5 manual dev-profile check (task 2.2).",
        "T6 task 3.1: run the chunk command, then the full floor."
      ]
    }
  ],
  "requirements": [
    {
      "shall": "### Requirement: TUN mode availability per backend The system SHALL offer TUN mode only when the selected backend is sing-box or xray. For the v2ray backend, TUN SHALL be unavailable because v2ray-core has no native TUN inbound. For xray, TUN SHALL additionally require Xray-core v26.1.13 or newer (the first release with the `tun` inbound); starting a TUN connection with an older xray SHALL fail before spawn with an error naming the installed and required versions. For xray versions in the range 26.1.13 through 26.6.22 — affected by the upstream TUN crash on quickly-closed connections (Xray-core #6364, fixed in 26.6.27) — the TUN start SHALL proceed but emit an advisory into the process log stream naming the installed version, the crash behavior, and the fixed version. When the installed xray version cannot be read or parsed, the start SHALL proceed and the process log SHALL contain a warning that the version could not be read and the minimum-version check was skipped.",
      "tests": [
        "v2ray-rs-process manager::tests::tun_start_fails_fast_on_pre_tun_xray_version",
        "v2ray-rs-process manager::tests::tun_start_warns_on_panic_affected_xray_version",
        "v2ray-rs-process manager::tests::xray_tun_start_warns_on_unreadable_version"
      ]
    },
    {
      "shall": "- **THEN** the enable toggle SHALL be insensitive with an explanatory note, and no tun inbound SHALL be generated even if a stale `enabled` flag is persisted",
      "tests": [
        "v2ray-rs-core config::v2ray::tests::test_v2ray_never_emits_tun_even_when_enabled",
        "existing, unchanged by this change"
      ]
    },
    {
      "shall": "- **THEN** the TUN enable toggle SHALL be available, subject to the capability gate",
      "tests": [
        "existing, unchanged by this change (ui preferences/tun.rs)"
      ]
    },
    {
      "shall": "- **THEN** the connection preflight SHALL fail with an error stating the installed version and the required minimum, without spawning the backend",
      "tests": [
        "v2ray-rs-process manager::tests::tun_start_fails_fast_on_pre_tun_xray_version"
      ]
    },
    {
      "shall": "- **THEN** the connection SHALL start normally and a warning log line SHALL appear in the process logs naming the installed version, the quickly-closed-connection crash, and 26.6.27 as the fixed version",
      "tests": [
        "v2ray-rs-process manager::tests::tun_start_warns_on_panic_affected_xray_version"
      ]
    },
    {
      "shall": "- **THEN** the start SHALL continue and the process log SHALL contain a warning that the version could not be read and the minimum-version check was skipped",
      "tests": [
        "v2ray-rs-process manager::tests::xray_tun_start_warns_on_unreadable_version"
      ]
    },
    {
      "shall": "### Requirement: sing-box minimum version is checked before spawn The system SHALL require sing-box 1.13.0 or newer, the oldest version the generated config targets. Starting a sing-box connection with an older installed version SHALL fail before the backend is spawned, with an error naming the installed and required versions, and SHALL end the connection attempt without trying further candidates. When the installed version cannot be read, the start SHALL proceed and a warning SHALL be written to the process log stream stating that the version could not be read and the minimum-version check was skipped.",
      "tests": [
        "v2ray-rs-process manager::tests::singbox_start_fails_before_check_on_old_version",
        "v2ray-rs-process manager::tests::singbox_start_proceeds_on_minimum_version",
        "v2ray-rs-process manager::tests::singbox_start_warns_on_unreadable_version",
        "v2ray-rs-process manager::tests::is_host_level_classifies_every_variant",
        "v2ray-rs-ui connection::tests::host_level_failure_stops_the_candidate_loop"
      ]
    },
    {
      "shall": "- **THEN** Connect SHALL fail without spawning a backend, and the error SHALL name 1.12.4 and 1.13.0",
      "tests": [
        "v2ray-rs-process manager::tests::singbox_start_fails_before_check_on_old_version"
      ]
    },
    {
      "shall": "- **THEN** the start SHALL proceed to the config check",
      "tests": [
        "v2ray-rs-process manager::tests::singbox_start_proceeds_on_minimum_version"
      ]
    },
    {
      "shall": "- **THEN** the start SHALL proceed and the process log SHALL contain a warning that the minimum-version check was skipped",
      "tests": [
        "v2ray-rs-process manager::tests::singbox_start_warns_on_unreadable_version"
      ]
    },
    {
      "shall": "### Requirement: v2ray and xray connects require referenced geodata When the selected backend is v2ray or xray and the enabled routing rules, the active imported-profile rules, or the DNS rules used for the connection reference a GeoIP or GeoSite tag, the system SHALL check before starting the connection that `geoip.dat` and `geosite.dat` exist in the profile's geodata directory. When either is missing, Connect SHALL fail without spawning a backend, and the user SHALL see an error stating that the rules need geodata that has not been downloaded, with an action that starts the geodata download for the current backend. sing-box connects SHALL NOT be blocked by missing rule-set files, which fall back to remote rule-sets.",
      "tests": [
        "v2ray-rs-ui app::tests::missing_geodata_xray_geosite_rule_without_file",
        "v2ray-rs-ui app::tests::missing_geodata_imported_profile_geo_rule",
        "v2ray-rs-ui app::tests::missing_geodata_custom_dns_geosite_rule",
        "v2ray-rs-ui app::tests::missing_geodata_one_file_missing",
        "v2ray-rs-ui app::tests::missing_geodata_singbox_is_never_gated"
      ]
    },
    {
      "shall": "- **THEN** Connect SHALL fail without spawning a backend and the error SHALL offer a \"Download geodata\" action",
      "tests": [
        "v2ray-rs-ui app::tests::missing_geodata_xray_geosite_rule_without_file",
        "manual: dev profile with geosite.dat removed (task 2.2)"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL download the geodata files for the current backend and report success or failure",
      "tests": [
        "manual: Download geodata toast action in dev profile (task 2.2)"
      ]
    },
    {
      "shall": "- **THEN** Connect SHALL proceed",
      "tests": [
        "v2ray-rs-ui app::tests::missing_geodata_without_geo_rules",
        "v2ray-rs-ui app::tests::missing_geodata_singbox_is_never_gated"
      ]
    },
    {
      "shall": "- **THEN** Connect SHALL proceed",
      "tests": [
        "v2ray-rs-ui app::tests::missing_geodata_without_geo_rules",
        "v2ray-rs-ui app::tests::missing_geodata_singbox_is_never_gated"
      ]
    }
  ],
  "testHarness": [
    "write_script — crates/process/src/manager.rs:fn write_script(dir: &std::path::Path, body: &str) -> PathBuf { — writes `<dir>/backend` as `#!/bin/sh\\n{body}`, mode 0755",
    "manager_for — crates/process/src/manager.rs:fn manager_for(dir: &tempfile::TempDir, script_body: &str) -> ProcessManager { — config.json `{}` + stub backend + ProcessManager::new(.., None geodata)",
    "VERSION_STUB — crates/process/src/manager.rs:const VERSION_STUB: &str = — shell prefix answering `version` with 'Xray 26.3.27 (Xray, Penetrates Everything.)'",
    "backend_log — crates/process/src/manager.rs:fn backend_log(dir: &std::path::Path) -> Arc<RotatingFileWriter> { — RotatingFileWriter at <dir>/backend.log (DEFAULT_MAX_BYTES)",
    "stub_helper — crates/process/src/manager.rs:fn stub_helper(dir: &std::path::Path, body: &str) -> (PathBuf, PathBuf) { — fake netctl appending $1 to <dir>/calls",
    "xray_on_lo — crates/process/src/manager.rs:fn xray_on_lo(helper: PathBuf) -> TunRuntime { — xray TunRuntime on iface lo, 172.19.0.1/30",
    "crashing_backend — crates/process/src/manager.rs:fn crashing_backend(dir: &std::path::Path, crashes: usize) -> String { — script body that exits 1 for the first N runs, then sleeps",
    "checks-file stub — crates/process/src/manager.rs:\"if [ \\\"$1\\\" = check ]; then echo check >> {}; exit 0; fi\\n{}\" — records each `check` call (respawn_skips_preflight); reuse for 'error before check runs'",
    "read_lines / wait_for_lines — crates/process/src/manager.rs:fn read_lines(path: &std::path::Path) -> Vec<String> { — file line readers (the async variant polls for n lines)",
    "drain_states — crates/process/src/manager.rs:fn drain_states(rx: &mut broadcast::Receiver<ProcessEvent>) -> Vec<ProcessState> { — collects StateChanged events",
    "inline fake-xray — crates/process/src/manager.rs:\"#!/bin/sh\\necho 'Xray 25.12.8 (Xray, Penetrates Everything.)'\\n\" — version-only binary + TunRuntime literal, in tun_start_fails_fast_on_pre_tun_xray_version / tun_start_warns_on_panic_affected_xray_version",
    "HostProbe — crates/process/src/manager.rs:pub struct HostProbe { — pins getcap/helper for TUN preflight; with_host_probe after with_tun",
    "Stub / stub — crates/ui/src/connection.rs:fn stub(script: &str) -> Stub { — tempdir, AppPaths::for_profile_in(AppProfile::Test), ensure_dirs, `<tmp>/backend` shell script",
    "executable — crates/ui/src/connection.rs:fn executable(dir: &std::path::Path, name: &str) -> PathBuf { — `exit 0` script",
    "candidate / node — crates/ui/src/connection.rs:fn candidate(address: &str) -> ConnectionCandidate { — Manual node ref over a Shadowsocks node at address:8388",
    "connect / connect_with — crates/ui/src/connection.rs:fn connect_with( — spawn_with(ConnectionRequest{..}, tx, configure) → (ConnectionHandle, relm4::Receiver<AppMsg>)",
    "singbox_settings / tun_settings — crates/ui/src/connection.rs:fn singbox_settings() -> AppSettings { — AppSettings default with SingBox backend / TUN enabled",
    "next_state / assert_nothing_after_terminal — crates/ui/src/connection.rs:async fn next_state( — waits RECV_TIMEOUT (20s) for ProcessStateConnection at GENERATION=7",
    "geo_rule / vless_node — crates/ui/src/geodata_service.rs:fn geo_rule(condition: RuleMatch, enabled: bool) -> RoutingRule { — RoutingRule with Proxy action; VLESS example.com:443 node for subscriptions",
    "app.rs tests — crates/ui/src/app.rs:fn error_toast_action_arms_only_for_matching_generation() { — plain #[test]s over pure fns; imports AutoResolveStrategy, BackendType, DnsConfig, ProxyNode, SubscriptionNode, TransportSettings, TunConfig, VlessConfig"
  ],
  "floor": "make test && make fmt && make clippy",
  "risks": [
    "Existing sing-box stubs do not answer `version`: stubs ending in `exec sleep 30` make the new probe wait the full 10 s CONFIG_CHECK_TIMEOUT per candidate start (ui connection.rs RECV_TIMEOUT = 20 s -> multi-candidate tests time out), and the manager.rs crash-count stub `echo run >> {runs}` would count the `version` exec as a crash run -> mitigation: codeTask a-T3/a-T4 add a `version` responder to every sing-box stub before implementing the gate; chunk run must finish well under 5 min.",
    "ui `live_connect_writes_backend_diagnostics` seeds `sing-box 1.11.0` and expects Running -> the new gate forces BackendTooOld; migrate stub and assertion to 1.13.0 (not a spec contradiction, a fixture predating the minimum).",
    "Concurrent change `warn-v2ray-tun-unsupported` MODIFIES the same tun-mode requirement 'TUN mode availability per backend' (its delta adds the v2ray toast + log line, lacks the unreadable-xray sentence; this delta adds the sentence, lacks the v2ray text) and likely edits `start_connection` in app.rs too -> land sequentially; the second to archive rebases its MODIFIED requirement text to include both additions, and the second implementer resolves the start_connection ordering (v2ray-TUN warning vs geodata block: geodata block first, since it prevents the connect).",
    "Error text change: xray TUN users lose the `has no TUN inbound` wording (now `installed xray X is too old; xray 26.1.13 or newer is required`) -> acceptable, the error still names installed and required per tun-mode scenario; no test or doc asserts the old text (workspace search: only manager.rs Display).",
    "sing-box 1.12.x users whose configs happened to load are now blocked -> clear message; constant is one line (design.md risk).",
    "ETXTBSY on the version probe is mitigated by spawn_with_etxtbsy_retry (same path as check_config).",
    "Geodata false negative: corrupt .dat passes the predicate -> backend check still rejects (design.md non-goal).",
    "Double-click / repeated connects can start two concurrent geodata downloads -> both write via atomic persist in core geodata; acceptable, no guard added (smallest change).",
    "Extracting `update_geodata` from network.rs touches Preferences Update Now -> keep its error strings and index build identical; manual check of Update Now in the same dev session.",
    "Accepted over-approximation: global routing rules and global DNS are checked even when every candidate uses an active imported profile that replaces them; the false positive is cleared by downloading geodata (one click). Smaller than per-candidate effective-config evaluation, which would change the task-2.1 signature."
  ],
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
