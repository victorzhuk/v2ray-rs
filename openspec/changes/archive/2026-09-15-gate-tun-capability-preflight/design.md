## Context

- `crates/process/src/manager.rs:191-244`: TUN preflight inside `start_with_connection`, per candidate. Capability gate only for the backend binary via `privilege::has_net_admin` in `spawn_blocking`; the route helper is first exercised by `xray-up` after the backend is already spawned (`manager.rs:373-388`).
- `crates/process/src/privilege.rs:42-59`: `has_net_admin` runs `getcap` with `std::process::Command::output()` — no timeout; a missing `getcap` returns `PrivilegeError::Spawn`, which the manager turns into `TunCapabilityProbe` and blocks the start. No euid check anywhere. `file_caps_supported` (`:354-359`) is used only by the grant (`:87-101`) and helper path resolution.
- `crates/process/src/tun.rs:183-186` `helper_needs_relogin` and the helper capability check are used only by the preferences page (`crates/ui/src/preferences/tun.rs:21-44`).
- `crates/ui/src/connection.rs:227-230`: any start `Err` is recorded and the loop fails over, so a host-level failure repeats per candidate (bounded to two by `stop-failover-on-shared-failure` when the text is identical — a separate pending change on the same loop).
- `crates/ui/src/app.rs:139-144`: every terminal `Error` becomes `Error: {msg}` in a plain toast. The grant button exists only in Preferences → TUN (`preferences/tun.rs:526-610`).
- The existing `ProcessError::TunBackendTooOld { installed }` variant (`manager.rs:66`) is the version-error class `is_host_level()` must classify as host-level; `check-backend-versions-geodata` later generalizes it to `BackendTooOld { backend, installed, required }`.

## Goals / Non-Goals

**Goals:**
- Every TUN prerequisite that can be checked without spawning — backend capabilities, helper existence/executability/capability, mount support — is checked before spawn, once per attempt.
- Host-level failures end the attempt once, and the failure the user can fix carries the fix: grant action, relogin, manual `setcap`.

**Non-Goals:**
- Reading file capabilities without `getcap` (would reimplement `vfs_cap_data` parsing; `setcap` for the grant needs libcap anyway).
- Changing the preferences page's own probes (they run on the GTK thread; separate change).
- Version gates and geodata preflight (`check-backend-versions-geodata`); recovery surfacing (`surface-tun-recovery-failures`).

## Decisions

- **Keep the preflight in `ProcessManager`, classify host-level errors.** `ProcessError` gains `is_host_level()` for capability, probe, helper, mount and version errors; `connection.rs` stops the loop on them, reports one `Error`, and emits `AppMsg::TunGrantRequired(generation)` first when the fix is the grant. Alternative rejected: a separate preflight pass in the connection task before the loop — duplicates the manager's gates, which respawn skipping and direct manager tests rely on.
- **Helper check (xray TUN only).** Resolved `helper_path()` must be absolute and exist (else `route helper v2ray-rs-netctl not found`); `access(X_OK)` must pass (relocated copy → `log out and back in to finish enabling TUN`, matching the preferences text); `has_net_admin(helper)` must be true (else grant). sing-box never runs the helper.
- **Root skips capability probes.** `geteuid() == 0` holds every capability; probing `getcap` there only adds a failure mode.
- **Bounded `getcap`, missing `getcap` still blocks.** Timeout 5 s, killed on expiry → `could not verify TUN capabilities: getcap timed out`. `ENOENT` → `getcap not found; install libcap to use TUN`. Alternative rejected: start anyway when `getcap` is missing — the spec forbids silently starting without privileges, and without libcap the grant (`setcap`) cannot work either.
- **`nosuid` backend at connect.** When `file_caps_supported(binary's dir)` is false the error is `PrivilegeError::Unsupported`'s text (manual `setcap` after moving the binary), not the grant action, because the grant would refuse the same path.
- **Grant action on connect.** `apply_state` shows the `Error` toast with a "Grant TUN privileges" button that opens Preferences on the TUN page when the preceding `TunGrantRequired` matched the current generation.

## Risks / Trade-offs

- [Helper `getcap` adds up to 5 s to an xray TUN connect on a wedged system] → same bound as the backend probe; runs once per attempt now instead of per candidate for the backend.
- [`stop-failover-on-shared-failure` edits the same `connection.rs` candidate loop] → different trigger (repeated identical text vs host-level class); whichever lands second rebases.
- [Version errors are classified via the existing `TunBackendTooOld` variant until `check-backend-versions-geodata` renames it] → `is_host_level()` tests over every variant keep the classification true across the rename.

## Plan appendix

```json
{
  "v": 2,
  "change": "gate-tun-capability-preflight",
  "baseSha": "b701fae583da52e8ea0e9c91944f2fb0721f9960",
  "generatedAt": "2026-09-15T12:31:39.851Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality",
    "sec"
  ],
  "chunks": [
    {
      "id": "probe-bound",
      "taskIds": [
        "1.1",
        "1.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": false,
      "seam": "privilege-probe-bound",
      "shard": "",
      "pkgDirs": [
        "crates/process/src"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/process/src/privilege.rs",
          "symbol": "has_net_admin",
          "anchor": "pub fn has_net_admin(path: &Path) -> Result<bool, PrivilegeError> {",
          "change": "Add 5 s timeout to getcap execution, killing child on expiry; map timeout to 'could not verify TUN capabilities: getcap timed out' and ENOENT to 'getcap not found; install libcap to use TUN'; extract pure probe_error_text helper and unit-test for timeout, not-found, and non-zero exit"
        },
        {
          "task": "1.2",
          "file": "crates/process/src/privilege.rs",
          "symbol": "caps_check_needed",
          "anchor": "enum GrantPlan {",
          "change": "Add pure caps_check_needed(euid: u32) -> bool returning euid != 0; unit test for euid 0 and 1000"
        }
      ],
      "contract": {
        "budgets": [
          "GETCAP_TIMEOUT = 5000 ms, kill on expiry — not abandon-on-drop",
          "probe_error_text must be allocation-light: two of three arms are 'static strings"
        ],
        "forbidden": [
          "root euid with any getcap child spawned",
          "timeout mapping to Ok (start must be blocked)",
          "ENOENT mapping to Ok(false)/Ok(true) or to the generic probe text",
          "timeout text != 'could not verify TUN capabilities: getcap timed out' (verbatim)",
          "not-found text != 'getcap not found; install libcap to use TUN' (verbatim)",
          "An unpinned Exit-arm probe message; the exact string is `could not verify TUN capabilities: getcap exited with {0}`"
        ],
        "seeding": [
          "All S1 tests are pure: probe_error_text(&ProbeFailure::...) and caps_check_needed(0u32/1000u32) — no filesystem, no getcap, no process; mirror privilege.rs:669-791 pure-test style",
          "Live has_net_admin is deliberately NOT exercised by tests (host getcap presence varies); its outcome mapping is the pure fn's contract"
        ],
        "states": [
          "caps-present",
          "caps-absent",
          "probe-timeout",
          "probe-missing",
          "probe-exit-failed",
          "check-needed",
          "check-skipped"
        ],
        "transitions": [
          {
            "effect": "set",
            "evidence": "crates/process/src/privilege.rs:42-59,317-332 (existing getcap_has_cap token match)",
            "input": "getcap exit 0, capset contains cap_net_admin",
            "state": "caps-present"
          },
          {
            "effect": "set",
            "evidence": "crates/process/src/privilege.rs:42-59 + manager.rs:225-244 (existing TunCapabilityMissing path)",
            "input": "getcap exit 0 without cap_net_admin",
            "state": "caps-absent"
          },
          {
            "effect": "forced",
            "evidence": "task 1.1 / spec scenario 'Capability probe hangs or is unavailable'; text must read exactly `could not verify TUN capabilities: getcap timed out`",
            "input": "getcap still running at 5000 ms",
            "state": "probe-timeout"
          },
          {
            "effect": "set",
            "evidence": "spec scenario 'Capability probe hangs or is unavailable' — text exactly `getcap not found; install libcap to use TUN`; start still blocked (no Ok fallback, setcap for the grant needs libcap anyway)",
            "input": "spawn error kind ENOENT (getcap not installed)",
            "state": "probe-missing"
          },
          {
            "effect": "set",
            "evidence": "crates/process/src/privilege.rs:24-26,45-52 (existing PrivilegeError::Probe stderr behavior kept)",
            "input": "getcap exits non-zero",
            "state": "probe-exit-failed"
          },
          {
            "effect": "clear",
            "evidence": "spec scenario 'Running as root'; closed decision 'Root skip'",
            "input": "caps_check_needed(0)",
            "state": "check-skipped"
          },
          {
            "effect": "set",
            "evidence": "task 1.2 pure predicate",
            "input": "caps_check_needed(1000)",
            "state": "check-needed"
          }
        ]
      },
      "redTasks": [],
      "codeTasks": [],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4",
      "coder": "rust-coder"
    },
    {
      "id": "manager-gates",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "prev": "probe-bound",
      "sharedPkg": "crates/process",
      "parallel": false,
      "seam": "manager-preflight-hostlevel",
      "shard": "",
      "pkgDirs": [
        "crates/process/src"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::start_with_connection",
          "anchor": "if self.backend == Some(BackendType::Xray)",
          "change": "For xray TUN before spawn: verify helper_path() is absolute and exists, check access(X_OK) mapping relocated failure to relogin error, check has_net_admin(helper); add new ProcessError variants; add stub tests with non-executable helper asserting relogin/permission error and no session record in backend log"
        },
        {
          "task": "2.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessManager::start_with_connection",
          "anchor": "let binary = self.binary_path.clone();",
          "change": "Add backend nosuid check via file_caps_supported on binary dir before capability probe, returning PrivilegeError::Unsupported text; add pure predicate test using synthetic /proc/self/mounts with mount_for_path"
        }
      ],
      "contract": {
        "budgets": [
          "each caps gate <= 5 s (S1 bound) — worst case one backend + one helper probe per start attempt",
          "all new gates run strictly before launch()/write_session_record (manager.rs:358-359) — zero cost after spawn"
        ],
        "forbidden": [
          "root euid with any getcap child spawned (backend or helper)",
          "helper gate evaluated for BackendType::SingBox",
          "any host-level failure with a `session` record in backend.log or a spawned child",
          "mount-unsupported error whose text offers the grant action (must carry PrivilegeError::Unsupported manual-setcap wording, privilege.rs:27-31)",
          "helper-relogin or helper-missing or probe or mount or version error triggering TunGrantRequired (S3: grant is only for missing capability)",
          "reordering that puts a getcap-dependent gate before the helper exists/X_OK gates"
        ],
        "seeding": [
          "Integration: ProcessManager::new(PathBuf::from('/bin/sh'), config, pid, None).with_tun(Some(TunRuntime{backend: Xray, helper_path: <tempfile under test TempDir>, ..})).with_backend(Xray) — exact pattern of tun_start_refuses_without_capability, manager.rs:1183-1222; /bin/sh backend is mandatory so the nosuid gate (checks /usr/bin) can never preempt the helper gates",
          "Non-executable helper: std::fs::write + Permissions::from_mode(0o644) (pattern connection.rs:466-468)",
          "No-session-record: .with_log_file(Some(backend_log(dir))) + wait_for_lines (manager.rs:885-914 helpers); assert zero `session` lines",
          "is_host_level: construct each ProcessError variant literal in a pure test — no manager, no filesystem",
          "nosuid predicate: mount_for_path(&synthetic_mounts, Path) — pure, pattern privilege.rs tests mount_lookup_picks_longest_prefix / nosuid_mount_is_unsupported (privilege.rs:693-758)",
          "Host tolerance: caps-dependent integration tests keep the existing disjunctive assertion style (manager.rs:1209-1216 accepts TunCapabilityMissing | TunCapabilityProbe) — getcap-less CI hosts get probe-missing, same host-level class"
        ],
        "states": [
          "preflight-pass",
          "mount-unsupported",
          "helper-missing",
          "helper-relogin",
          "helper-cap-missing",
          "backend-cap-missing",
          "probe-failed",
          "version-too-old",
          "root-skip"
        ],
        "transitions": [
          {
            "effect": "set",
            "evidence": "crates/process/src/manager.rs:47-74 (enum site); requirement tasks 2.1/2.2 + spec scenarios 'Route helper missing', 'Backend on a nosuid mount'",
            "input": "xray TUN, file_caps_supported(binary_path.parent()) == false",
            "state": "mount-unsupported"
          },
          {
            "effect": "set",
            "evidence": "spec scenario 'Route helper not executable yet'; text matches preferences wording via tun.rs:181-186 helper_needs_relogin",
            "input": "xray TUN, helper_path() not absolute or !exists",
            "state": "helper-missing"
          },
          {
            "effect": "set",
            "evidence": "spec scenario 'Route helper not executable yet'; access(X_OK) via nix (tun.rs:188-191)",
            "input": "xray TUN, helper exists, access(X_OK) fails (relocated copy, group not picked up)",
            "state": "helper-relogin"
          },
          {
            "effect": "set",
            "evidence": "spec scenario 'Route helper lacks its capability'; tun.rs:60-63 helper_path",
            "input": "xray TUN, non-root, helper X_OK ok, getcap(helper) lacks cap_net_admin",
            "state": "helper-cap-missing"
          },
          {
            "effect": "set",
            "evidence": "manager.rs:214-244 unchanged",
            "input": "xray TUN, non-root, backend getcap lacks cap_net_admin",
            "state": "backend-cap-missing"
          },
          {
            "effect": "set",
            "evidence": "S1 seam; surfaces as ProcessError::TunCapabilityProbe(payload = probe_error_text(...))",
            "input": "any getcap probe times out or getcap missing (non-root)",
            "state": "probe-failed"
          },
          {
            "effect": "no-op",
            "evidence": "spec scenario 'Running as root'; caps_check_needed(0) == false (S1); helper existence/X_OK still enforced",
            "input": "euid == 0, any caps gate",
            "state": "root-skip"
          },
          {
            "effect": "no-op",
            "evidence": "closed decision 'sing-box never runs the helper'; helper invocation is xray-only (manager.rs:373-388 xray-up flow)",
            "input": "sing-box TUN, any helper state",
            "state": "preflight-pass"
          },
          {
            "effect": "set",
            "evidence": "manager.rs:191-213 existing gate; is_host_level() true for TunBackendTooOld (manager.rs:66-69) — the version class until check-backend-versions-geodata generalizes it",
            "input": "xray < XRAY_TUN_MIN_VERSION with TUN on",
            "state": "version-too-old"
          },
          {
            "effect": "clear",
            "evidence": "manager.rs:214-244 flow with new gates inserted before it; spec scenario 'Missing capabilities block TUN start'",
            "input": "all gates pass (or root)",
            "state": "preflight-pass"
          },
          {
            "effect": "set",
            "evidence": "task 2.3 + closed decision is_host_level(); false for ConfigCheck (manager.rs:72-74), Spawn/io (manager.rs:49-50), TunDeviceTimeout/TunHelper readiness (manager.rs:60-64), and every other variant (BinaryNotFound, ConfigMissing, Wait, Transition) — per-candidate failover keeps running for all of them",
            "input": "ProcessError::is_host_level() over TunCapabilityMissing, TunCapabilityProbe, TunHelperMissing, TunHelperRelogin, TunHelperCapabilityMissing, TunMountUnsupported, TunBackendTooOld",
            "state": "preflight-pass"
          }
        ]
      },
      "redTasks": [],
      "codeTasks": [],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4",
      "coder": "rust-coder"
    },
    {
      "id": "host-level-classifier",
      "taskIds": [
        "2.3"
      ],
      "prev": "manager-gates",
      "sharedPkg": "crates/process",
      "parallel": false,
      "seam": "manager-preflight-hostlevel",
      "shard": "",
      "pkgDirs": [
        "crates/process/src"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "2.3",
          "file": "crates/process/src/manager.rs",
          "symbol": "ProcessError::is_host_level",
          "anchor": "pub enum ProcessError {",
          "change": "Implement is_host_level(&self) -> bool returning true for capability, probe, helper, mount, and version (TunBackendTooOld) errors, and false for config check, spawn, and readiness errors; add exhaustive unit test across all ProcessError variants"
        }
      ],
      "contract": {
        "budgets": [
          "each caps gate <= 5 s (S1 bound) — worst case one backend + one helper probe per start attempt",
          "all new gates run strictly before launch()/write_session_record (manager.rs:358-359) — zero cost after spawn"
        ],
        "forbidden": [
          "root euid with any getcap child spawned (backend or helper)",
          "helper gate evaluated for BackendType::SingBox",
          "any host-level failure with a `session` record in backend.log or a spawned child",
          "mount-unsupported error whose text offers the grant action (must carry PrivilegeError::Unsupported manual-setcap wording, privilege.rs:27-31)",
          "helper-relogin or helper-missing or probe or mount or version error triggering TunGrantRequired (S3: grant is only for missing capability)",
          "reordering that puts a getcap-dependent gate before the helper exists/X_OK gates"
        ],
        "seeding": [
          "Integration: ProcessManager::new(PathBuf::from('/bin/sh'), config, pid, None).with_tun(Some(TunRuntime{backend: Xray, helper_path: <tempfile under test TempDir>, ..})).with_backend(Xray) — exact pattern of tun_start_refuses_without_capability, manager.rs:1183-1222; /bin/sh backend is mandatory so the nosuid gate (checks /usr/bin) can never preempt the helper gates",
          "Non-executable helper: std::fs::write + Permissions::from_mode(0o644) (pattern connection.rs:466-468)",
          "No-session-record: .with_log_file(Some(backend_log(dir))) + wait_for_lines (manager.rs:885-914 helpers); assert zero `session` lines",
          "is_host_level: construct each ProcessError variant literal in a pure test — no manager, no filesystem",
          "nosuid predicate: mount_for_path(&synthetic_mounts, Path) — pure, pattern privilege.rs tests mount_lookup_picks_longest_prefix / nosuid_mount_is_unsupported (privilege.rs:693-758)",
          "Host tolerance: caps-dependent integration tests keep the existing disjunctive assertion style (manager.rs:1209-1216 accepts TunCapabilityMissing | TunCapabilityProbe) — getcap-less CI hosts get probe-missing, same host-level class"
        ],
        "states": [
          "preflight-pass",
          "mount-unsupported",
          "helper-missing",
          "helper-relogin",
          "helper-cap-missing",
          "backend-cap-missing",
          "probe-failed",
          "version-too-old",
          "root-skip"
        ],
        "transitions": [
          {
            "effect": "set",
            "evidence": "crates/process/src/manager.rs:47-74 (enum site); requirement tasks 2.1/2.2 + spec scenarios 'Route helper missing', 'Backend on a nosuid mount'",
            "input": "xray TUN, file_caps_supported(binary_path.parent()) == false",
            "state": "mount-unsupported"
          },
          {
            "effect": "set",
            "evidence": "spec scenario 'Route helper not executable yet'; text matches preferences wording via tun.rs:181-186 helper_needs_relogin",
            "input": "xray TUN, helper_path() not absolute or !exists",
            "state": "helper-missing"
          },
          {
            "effect": "set",
            "evidence": "spec scenario 'Route helper not executable yet'; access(X_OK) via nix (tun.rs:188-191)",
            "input": "xray TUN, helper exists, access(X_OK) fails (relocated copy, group not picked up)",
            "state": "helper-relogin"
          },
          {
            "effect": "set",
            "evidence": "spec scenario 'Route helper lacks its capability'; tun.rs:60-63 helper_path",
            "input": "xray TUN, non-root, helper X_OK ok, getcap(helper) lacks cap_net_admin",
            "state": "helper-cap-missing"
          },
          {
            "effect": "set",
            "evidence": "manager.rs:214-244 unchanged",
            "input": "xray TUN, non-root, backend getcap lacks cap_net_admin",
            "state": "backend-cap-missing"
          },
          {
            "effect": "set",
            "evidence": "S1 seam; surfaces as ProcessError::TunCapabilityProbe(payload = probe_error_text(...))",
            "input": "any getcap probe times out or getcap missing (non-root)",
            "state": "probe-failed"
          },
          {
            "effect": "no-op",
            "evidence": "spec scenario 'Running as root'; caps_check_needed(0) == false (S1); helper existence/X_OK still enforced",
            "input": "euid == 0, any caps gate",
            "state": "root-skip"
          },
          {
            "effect": "no-op",
            "evidence": "closed decision 'sing-box never runs the helper'; helper invocation is xray-only (manager.rs:373-388 xray-up flow)",
            "input": "sing-box TUN, any helper state",
            "state": "preflight-pass"
          },
          {
            "effect": "set",
            "evidence": "manager.rs:191-213 existing gate; is_host_level() true for TunBackendTooOld (manager.rs:66-69) — the version class until check-backend-versions-geodata generalizes it",
            "input": "xray < XRAY_TUN_MIN_VERSION with TUN on",
            "state": "version-too-old"
          },
          {
            "effect": "clear",
            "evidence": "manager.rs:214-244 flow with new gates inserted before it; spec scenario 'Missing capabilities block TUN start'",
            "input": "all gates pass (or root)",
            "state": "preflight-pass"
          },
          {
            "effect": "set",
            "evidence": "task 2.3 + closed decision is_host_level(); false for ConfigCheck (manager.rs:72-74), Spawn/io (manager.rs:49-50), TunDeviceTimeout/TunHelper readiness (manager.rs:60-64), and every other variant (BinaryNotFound, ConfigMissing, Wait, Transition) — per-candidate failover keeps running for all of them",
            "input": "ProcessError::is_host_level() over TunCapabilityMissing, TunCapabilityProbe, TunHelperMissing, TunHelperRelogin, TunHelperCapabilityMissing, TunMountUnsupported, TunBackendTooOld",
            "state": "preflight-pass"
          }
        ]
      },
      "redTasks": [],
      "codeTasks": [],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4",
      "coder": "rust-coder"
    },
    {
      "id": "connection-loop-stop",
      "taskIds": [
        "3.1"
      ],
      "prev": "host-level-classifier",
      "sharedPkg": "crates/ui",
      "parallel": false,
      "seam": "connection-loop-stop",
      "shard": "",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "3.1",
          "file": "crates/ui/src/connection.rs",
          "symbol": "spawn",
          "anchor": "failures.push(format!(\"{candidate_label}: {e}\"));",
          "change": "When start_with_connection returns Err(e) with e.is_host_level(): emit AppMsg::TunGrantRequired(generation) if error indicates missing capability, report terminal ProcessState::Error(e.to_string()), and break candidate loop; add stub test with two candidates and unprivileged backend verifying one start attempt and one terminal Error"
        },
        {
          "task": "3.1",
          "file": "crates/ui/src/app.rs",
          "symbol": "AppMsg",
          "anchor": "pub enum AppMsg",
          "change": "Add AppMsg::TunGrantRequired(u64) variant only"
        }
      ],
      "contract": {
        "budgets": [
          "host-level failure bounds the attempt to exactly 1 start_with_connection call (first failing candidate)",
          "worst added wall time per attempt: two 5 s getcap probes (backend + helper), root: 0"
        ],
        "forbidden": [
          "host-level error followed by a second start attempt (no second spawn/config-write for candidate 2)",
          "host-level error wrapped in 'All candidates failed' / summarize_failures (connection.rs:317-318, 420-435)",
          "TunGrantRequired emitted after the terminal Error, or for probe/relogin/missing-helper/mount/version errors",
          "new AppMsg::TunGrantRequired after the terminal report (channel must close clean: assert_nothing_after_terminal)"
        ],
        "seeding": [
          "Stub harness: stub() + connect(&stub, settings, vec![candidate(a), candidate(b)]) with settings.backend.backend_type = Xray and settings.tun.enabled = true (singbox_settings pattern, connection.rs:507-511; build_tun_runtime turns it into a TunRuntime)",
          "Terminal state via next_state(rx) + assert_nothing_after_terminal (connection.rs:513-548)",
          "One-attempt observable: Error text contains candidate-1 label, not candidate-2 label, and no 'All candidates failed' prefix (host-level terminal uses e.to_string() alone)",
          "Host tolerance (getcap-less or nosuid-/tmp hosts yield TunCapabilityProbe or TunMountUnsupported instead of TunCapabilityMissing): the loop-stop assertions hold for every host-level variant; do NOT assert TunGrantRequired unconditionally — assert it only when observed, and always before the Error",
          "Pure grant_fixable rows construct ProcessError variants directly — no process, no manager"
        ],
        "states": [
          "looping",
          "host-stopped",
          "exhausted",
          "running",
          "user-stopped"
        ],
        "transitions": [
          {
            "effect": "forced",
            "evidence": "crates/ui/src/connection.rs:215-231 (current Err arm pushes failure and continues) — change target; spec scenario 'Missing capabilities block TUN start: SHALL NOT try further candidates'",
            "input": "start Err(e) with e.is_host_level()",
            "state": "host-stopped"
          },
          {
            "effect": "set",
            "evidence": "task 3.1 + spec scenario 'Missing capabilities block TUN start / Route helper lacks its capability'; grant set is exactly TunCapabilityMissing + TunHelperCapabilityMissing (S2 variants); report closure connection.rs:86-90",
            "input": "host-level Err and grant_fixable(e)",
            "state": "host-stopped"
          },
          {
            "effect": "clear",
            "evidence": "connection.rs:317-318 terminal report must carry the error's own text, not summarize_failures",
            "input": "host-level Err (any variant)",
            "state": "host-stopped"
          },
          {
            "effect": "set",
            "evidence": "spec scenarios: relogin/mount/probe/version fixes are not the grant; S2 forbidden list",
            "input": "host-level Err not grant-fixable (probe, relogin, helper-missing, mount, version)",
            "state": "host-stopped"
          },
          {
            "effect": "no-op",
            "evidence": "connection.rs:227-231 unchanged — regression-protected by last_candidate_failure_reports_one_error (connection.rs:562-608)",
            "input": "start Err(e) non-host-level (config check, spawn, crash budget)",
            "state": "looping"
          },
          {
            "effect": "no-op",
            "evidence": "connection.rs:126-133 Stop path and 317-318 exhausted path unchanged",
            "input": "Stop cmd mid-loop / candidates exhausted",
            "state": "user-stopped / exhausted"
          }
        ]
      },
      "redTasks": [],
      "codeTasks": [],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4",
      "coder": "rust-coder"
    },
    {
      "id": "grant-toast-action",
      "taskIds": [
        "3.2",
        "4.1"
      ],
      "prev": "connection-loop-stop",
      "sharedPkg": "crates/ui",
      "parallel": false,
      "seam": "grant-toast-action",
      "shard": "",
      "pkgDirs": [
        "crates/ui/src"
      ],
      "pkgs": [],
      "sites": [
        {
          "task": "3.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "App::apply_state",
          "anchor": "fn apply_state(&mut self, state: &ProcessState) {",
          "change": "Track grant_generation from AppMsg::TunGrantRequired(u64); build the actionable Error toast where the sender is in scope (the ProcessStateConnection arm, app.rs ~1189) — apply_state has &mut self only and cannot emit; decide via pure error_toast_action(generation, grant_generation) -> Option<ToastAction>"
        }
      ],
      "contract": {
        "budgets": [
          "toast decision is O(1) pure comparison — no GTK in the predicate; only the toast build touches widgets"
        ],
        "forbidden": [
          "Grant TUN privileges button shown without a matching current-generation TunGrantRequired",
          "button attached to a non-Error toast",
          "grant_generation surviving after the Error toast is shown, or surviving a generation advance",
          "stale-generation TunGrantRequired arming the toast (must be dropped like stale states, app.rs:1189-1195)"
        ],
        "seeding": [
          "Pure unit test only: error_toast_action with literal u64 generations and Option — no GTK init, no component; pattern of app.rs:2034-2053 pure-helper tests",
          "Toast wiring is verified by task 4.2 live manual check (remove helper caps -> connect xray TUN -> toast button opens Preferences on TUN page); no automated UI test — repo has none for toasts"
        ],
        "states": [
          "grant-idle",
          "grant-armed",
          "toast-with-action"
        ],
        "transitions": [
          {
            "effect": "set",
            "evidence": "crates/ui/src/app.rs:1189-1195 generation-gate pattern + is_current_generation app.rs:1515-1517; AppMsg enum app.rs:104-131",
            "input": "AppMsg::TunGrantRequired(g), is_current_generation(g, connection_generation)",
            "state": "grant-armed"
          },
          {
            "effect": "no-op",
            "evidence": "superseded-connection handling precedent app.rs:1190-1195, 1291-1296",
            "input": "AppMsg::TunGrantRequired(g), g != connection_generation",
            "state": "grant-idle"
          },
          {
            "effect": "set",
            "evidence": "task 3.2 + design.md decision 'Grant action on connect'; toast built in the ProcessStateConnection arm where sender is in scope (app.rs:1189), apply_state app.rs:139-146 keeps the plain toast",
            "input": "ProcessState::Error for current generation while grant-armed",
            "state": "toast-with-action"
          },
          {
            "effect": "set",
            "evidence": "task 3.2; button handler -> AppMsg::OpenPreferences (app.rs:1313-1345) + TUN page visible; preferences page built at preferences/mod.rs:75-83 via build_tun_page (preferences/tun.rs:47-50)",
            "input": "user clicks 'Grant TUN privileges'",
            "state": "toast-with-action"
          },
          {
            "effect": "set",
            "evidence": "pure error_toast_action: Some(ToastAction::GrantTun) iff grant_generation == Some(generation), else None — every other Error keeps today's plain toast (app.rs:142-144)",
            "input": "ProcessState::Error with no armed grant or stale generation",
            "state": "grant-idle"
          }
        ]
      },
      "redTasks": [],
      "codeTasks": [],
      "redTests": [],
      "redRun": "",
      "verify": "timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-ui -- --test-threads=4",
      "coder": "rust-coder"
    }
  ],
  "seams": [
    {
      "id": "privilege-probe-bound",
      "tasks": [
        "1.1",
        "1.2"
      ],
      "summary": "NO-RED-WAIVER: rust-coder writes tests and code in one dispatch. Sites: crates/process/src/privilege.rs only (tests inline in its #[cfg(test)] mod tests — the sole test file this chunk touches). Bound the getcap probe, name a missing libcap, add the pure euid gate. NO-TESTER-WAIVER: chunk closes by waiver after guard; verification = the chunk's cargo test command plus the run floor.",
      "contract": {
        "budgets": [
          "GETCAP_TIMEOUT = 5000 ms, kill on expiry — not abandon-on-drop",
          "probe_error_text must be allocation-light: two of three arms are 'static strings"
        ],
        "forbidden": [
          "root euid with any getcap child spawned",
          "timeout mapping to Ok (start must be blocked)",
          "ENOENT mapping to Ok(false)/Ok(true) or to the generic probe text",
          "timeout text != 'could not verify TUN capabilities: getcap timed out' (verbatim)",
          "not-found text != 'getcap not found; install libcap to use TUN' (verbatim)",
          "An unpinned Exit-arm probe message; the exact string is `could not verify TUN capabilities: getcap exited with {0}`"
        ],
        "seeding": [
          "All S1 tests are pure: probe_error_text(&ProbeFailure::...) and caps_check_needed(0u32/1000u32) — no filesystem, no getcap, no process; mirror privilege.rs:669-791 pure-test style",
          "Live has_net_admin is deliberately NOT exercised by tests (host getcap presence varies); its outcome mapping is the pure fn's contract"
        ],
        "states": [
          "caps-present",
          "caps-absent",
          "probe-timeout",
          "probe-missing",
          "probe-exit-failed",
          "check-needed",
          "check-skipped"
        ],
        "transitions": [
          {
            "effect": "set",
            "evidence": "crates/process/src/privilege.rs:42-59,317-332 (existing getcap_has_cap token match)",
            "input": "getcap exit 0, capset contains cap_net_admin",
            "state": "caps-present"
          },
          {
            "effect": "set",
            "evidence": "crates/process/src/privilege.rs:42-59 + manager.rs:225-244 (existing TunCapabilityMissing path)",
            "input": "getcap exit 0 without cap_net_admin",
            "state": "caps-absent"
          },
          {
            "effect": "forced",
            "evidence": "task 1.1 / spec scenario 'Capability probe hangs or is unavailable'; text must read exactly `could not verify TUN capabilities: getcap timed out`",
            "input": "getcap still running at 5000 ms",
            "state": "probe-timeout"
          },
          {
            "effect": "set",
            "evidence": "spec scenario 'Capability probe hangs or is unavailable' — text exactly `getcap not found; install libcap to use TUN`; start still blocked (no Ok fallback, setcap for the grant needs libcap anyway)",
            "input": "spawn error kind ENOENT (getcap not installed)",
            "state": "probe-missing"
          },
          {
            "effect": "set",
            "evidence": "crates/process/src/privilege.rs:24-26,45-52 (existing PrivilegeError::Probe stderr behavior kept)",
            "input": "getcap exits non-zero",
            "state": "probe-exit-failed"
          },
          {
            "effect": "clear",
            "evidence": "spec scenario 'Running as root'; closed decision 'Root skip'",
            "input": "caps_check_needed(0)",
            "state": "check-skipped"
          },
          {
            "effect": "set",
            "evidence": "task 1.2 pure predicate",
            "input": "caps_check_needed(1000)",
            "state": "check-needed"
          }
        ]
      },
      "codeTasks": [
        "FIRST — tests in crates/process/src/privilege.rs #[cfg(test)] mod tests (the only test file this chunk may add/change): probe_error_text rows for ProbeFailure::Timeout / NotFound / Exit(String) asserting the three exact strings; caps_check_needed(0) == false and caps_check_needed(1000) == true (Exit arm pinned verbatim: `could not verify TUN capabilities: getcap exited with {0}`)",
        "privilege.rs: define pub(crate) ProbeFailure and pure probe_error_text(&ProbeFailure) -> String; wire has_net_admin to spawn getcap (not .output()), kill at GETCAP_TIMEOUT, map NotFound (ENOENT) / Timeout / Exit(stderr.trim()) through it",
        "privilege.rs: pure pub(crate) caps_check_needed(euid: u32) -> bool",
        "Keep has_net_admin signature Result<bool, PrivilegeError> so manager call sites and lib.rs:14 export stay untouched"
      ]
    },
    {
      "id": "manager-preflight-hostlevel",
      "tasks": [
        "2.1",
        "2.2",
        "2.3"
      ],
      "summary": "NO-RED-WAIVER: rust-coder writes tests and code in one dispatch. Sites: crates/process/src/manager.rs (+ its inline #[cfg(test)] mod tests) for gates, variants, is_host_level; crates/process/src/privilege.rs only for the synthetic-mount pure test row (its inline tests). Gate order pinned: version -> nosuid(backend dir) -> helper exists/absolute -> helper X_OK -> backend caps -> helper caps -> config check -> session record+spawn. All gates before launch() so no `session` record and no child on failure. NO-TESTER-WAIVER: chunk closes by waiver after guard; verification = the chunk's cargo test command plus the run floor.",
      "contract": {
        "budgets": [
          "each caps gate <= 5 s (S1 bound) — worst case one backend + one helper probe per start attempt",
          "all new gates run strictly before launch()/write_session_record (manager.rs:358-359) — zero cost after spawn"
        ],
        "forbidden": [
          "root euid with any getcap child spawned (backend or helper)",
          "helper gate evaluated for BackendType::SingBox",
          "any host-level failure with a `session` record in backend.log or a spawned child",
          "mount-unsupported error whose text offers the grant action (must carry PrivilegeError::Unsupported manual-setcap wording, privilege.rs:27-31)",
          "helper-relogin or helper-missing or probe or mount or version error triggering TunGrantRequired (S3: grant is only for missing capability)",
          "reordering that puts a getcap-dependent gate before the helper exists/X_OK gates"
        ],
        "seeding": [
          "Integration: ProcessManager::new(PathBuf::from('/bin/sh'), config, pid, None).with_tun(Some(TunRuntime{backend: Xray, helper_path: <tempfile under test TempDir>, ..})).with_backend(Xray) — exact pattern of tun_start_refuses_without_capability, manager.rs:1183-1222; /bin/sh backend is mandatory so the nosuid gate (checks /usr/bin) can never preempt the helper gates",
          "Non-executable helper: std::fs::write + Permissions::from_mode(0o644) (pattern connection.rs:466-468)",
          "No-session-record: .with_log_file(Some(backend_log(dir))) + wait_for_lines (manager.rs:885-914 helpers); assert zero `session` lines",
          "is_host_level: construct each ProcessError variant literal in a pure test — no manager, no filesystem",
          "nosuid predicate: mount_for_path(&synthetic_mounts, Path) — pure, pattern privilege.rs tests mount_lookup_picks_longest_prefix / nosuid_mount_is_unsupported (privilege.rs:693-758)",
          "Host tolerance: caps-dependent integration tests keep the existing disjunctive assertion style (manager.rs:1209-1216 accepts TunCapabilityMissing | TunCapabilityProbe) — getcap-less CI hosts get probe-missing, same host-level class"
        ],
        "states": [
          "preflight-pass",
          "mount-unsupported",
          "helper-missing",
          "helper-relogin",
          "helper-cap-missing",
          "backend-cap-missing",
          "probe-failed",
          "version-too-old",
          "root-skip"
        ],
        "transitions": [
          {
            "effect": "set",
            "evidence": "crates/process/src/manager.rs:47-74 (enum site); requirement tasks 2.1/2.2 + spec scenarios 'Route helper missing', 'Backend on a nosuid mount'",
            "input": "xray TUN, file_caps_supported(binary_path.parent()) == false",
            "state": "mount-unsupported"
          },
          {
            "effect": "set",
            "evidence": "spec scenario 'Route helper not executable yet'; text matches preferences wording via tun.rs:181-186 helper_needs_relogin",
            "input": "xray TUN, helper_path() not absolute or !exists",
            "state": "helper-missing"
          },
          {
            "effect": "set",
            "evidence": "spec scenario 'Route helper not executable yet'; access(X_OK) via nix (tun.rs:188-191)",
            "input": "xray TUN, helper exists, access(X_OK) fails (relocated copy, group not picked up)",
            "state": "helper-relogin"
          },
          {
            "effect": "set",
            "evidence": "spec scenario 'Route helper lacks its capability'; tun.rs:60-63 helper_path",
            "input": "xray TUN, non-root, helper X_OK ok, getcap(helper) lacks cap_net_admin",
            "state": "helper-cap-missing"
          },
          {
            "effect": "set",
            "evidence": "manager.rs:214-244 unchanged",
            "input": "xray TUN, non-root, backend getcap lacks cap_net_admin",
            "state": "backend-cap-missing"
          },
          {
            "effect": "set",
            "evidence": "S1 seam; surfaces as ProcessError::TunCapabilityProbe(payload = probe_error_text(...))",
            "input": "any getcap probe times out or getcap missing (non-root)",
            "state": "probe-failed"
          },
          {
            "effect": "no-op",
            "evidence": "spec scenario 'Running as root'; caps_check_needed(0) == false (S1); helper existence/X_OK still enforced",
            "input": "euid == 0, any caps gate",
            "state": "root-skip"
          },
          {
            "effect": "no-op",
            "evidence": "closed decision 'sing-box never runs the helper'; helper invocation is xray-only (manager.rs:373-388 xray-up flow)",
            "input": "sing-box TUN, any helper state",
            "state": "preflight-pass"
          },
          {
            "effect": "set",
            "evidence": "manager.rs:191-213 existing gate; is_host_level() true for TunBackendTooOld (manager.rs:66-69) — the version class until check-backend-versions-geodata generalizes it",
            "input": "xray < XRAY_TUN_MIN_VERSION with TUN on",
            "state": "version-too-old"
          },
          {
            "effect": "clear",
            "evidence": "manager.rs:214-244 flow with new gates inserted before it; spec scenario 'Missing capabilities block TUN start'",
            "input": "all gates pass (or root)",
            "state": "preflight-pass"
          },
          {
            "effect": "set",
            "evidence": "task 2.3 + closed decision is_host_level(); false for ConfigCheck (manager.rs:72-74), Spawn/io (manager.rs:49-50), TunDeviceTimeout/TunHelper readiness (manager.rs:60-64), and every other variant (BinaryNotFound, ConfigMissing, Wait, Transition) — per-candidate failover keeps running for all of them",
            "input": "ProcessError::is_host_level() over TunCapabilityMissing, TunCapabilityProbe, TunHelperMissing, TunHelperRelogin, TunHelperCapabilityMissing, TunMountUnsupported, TunBackendTooOld",
            "state": "preflight-pass"
          }
        ]
      },
      "codeTasks": [
        "FIRST — tests in crates/process/src/manager.rs #[cfg(test)] mod tests: TunHelperMissing (missing temp helper, /bin/sh backend, with_log_file: no `session` line, no child); TunHelperRelogin (0o644 temp helper); sing-box runtime + missing helper passes the helper gate; is_host_level() row per ProcessError variant (all 12+; version row via TunBackendTooOld, false rows incl. Spawn/ConfigCheck/TunDeviceTimeout/TunHelper); plus pure mount-unsupported text test in crates/process/src/privilege.rs tests extending the mount_for_path synthetic-mount rows (nosuid opts -> PrivilegeError::Unsupported display carried into the ProcessError text)",
        "manager.rs ProcessError: add TunHelperMissing, TunHelperRelogin, TunHelperCapabilityMissing(PathBuf), TunMountUnsupported(String); add pub fn is_host_level(&self) -> bool with the classification table",
        "manager.rs start_with_connection TUN block: insert gate order version -> nosuid(backend dir) -> helper-exists -> helper-X_OK -> backend-caps -> helper-caps; wrap both caps gates in caps_check_needed(geteuid()) (nix::unistd::geteuid); gate euid call once per start",
        "Helper gates run only when self.backend == Some(BackendType::Xray); reuse nix access(X_OK) semantics (tun.rs:188-191 pattern); helper_path() from v2ray_rs_process::tun (lib.rs:18-21) — do not re-resolve",
        "Relogin text exactly `log out and back in to finish enabling TUN`; missing-helper text exactly `route helper v2ray-rs-netctl not found`; TunMountUnsupported payload = PrivilegeError::Unsupported Display (privilege.rs:27-31)"
      ]
    },
    {
      "id": "connection-loop-stop",
      "tasks": [
        "3.1"
      ],
      "summary": "NO-RED-WAIVER: rust-coder writes tests and code in one dispatch. Sites: crates/ui/src/connection.rs (+ its inline #[cfg(test)] mod tests) for the loop change; crates/ui/src/app.rs AppMsg enum only for the new variant TunGrantRequired(u64) (S4 handles it). Loop stops on host-level errors with one terminal Error carrying the raw error text; TunGrantRequired(generation) emitted first when the fix is the grant. NO-TESTER-WAIVER: chunk closes by waiver after guard; verification = the chunk's cargo test command plus the run floor.",
      "contract": {
        "budgets": [
          "host-level failure bounds the attempt to exactly 1 start_with_connection call (first failing candidate)",
          "worst added wall time per attempt: two 5 s getcap probes (backend + helper), root: 0"
        ],
        "forbidden": [
          "host-level error followed by a second start attempt (no second spawn/config-write for candidate 2)",
          "host-level error wrapped in 'All candidates failed' / summarize_failures (connection.rs:317-318, 420-435)",
          "TunGrantRequired emitted after the terminal Error, or for probe/relogin/missing-helper/mount/version errors",
          "new AppMsg::TunGrantRequired after the terminal report (channel must close clean: assert_nothing_after_terminal)"
        ],
        "seeding": [
          "Stub harness: stub() + connect(&stub, settings, vec![candidate(a), candidate(b)]) with settings.backend.backend_type = Xray and settings.tun.enabled = true (singbox_settings pattern, connection.rs:507-511; build_tun_runtime turns it into a TunRuntime)",
          "Terminal state via next_state(rx) + assert_nothing_after_terminal (connection.rs:513-548)",
          "One-attempt observable: Error text contains candidate-1 label, not candidate-2 label, and no 'All candidates failed' prefix (host-level terminal uses e.to_string() alone)",
          "Host tolerance (getcap-less or nosuid-/tmp hosts yield TunCapabilityProbe or TunMountUnsupported instead of TunCapabilityMissing): the loop-stop assertions hold for every host-level variant; do NOT assert TunGrantRequired unconditionally — assert it only when observed, and always before the Error",
          "Pure grant_fixable rows construct ProcessError variants directly — no process, no manager"
        ],
        "states": [
          "looping",
          "host-stopped",
          "exhausted",
          "running",
          "user-stopped"
        ],
        "transitions": [
          {
            "effect": "forced",
            "evidence": "crates/ui/src/connection.rs:215-231 (current Err arm pushes failure and continues) — change target; spec scenario 'Missing capabilities block TUN start: SHALL NOT try further candidates'",
            "input": "start Err(e) with e.is_host_level()",
            "state": "host-stopped"
          },
          {
            "effect": "set",
            "evidence": "task 3.1 + spec scenario 'Missing capabilities block TUN start / Route helper lacks its capability'; grant set is exactly TunCapabilityMissing + TunHelperCapabilityMissing (S2 variants); report closure connection.rs:86-90",
            "input": "host-level Err and grant_fixable(e)",
            "state": "host-stopped"
          },
          {
            "effect": "clear",
            "evidence": "connection.rs:317-318 terminal report must carry the error's own text, not summarize_failures",
            "input": "host-level Err (any variant)",
            "state": "host-stopped"
          },
          {
            "effect": "set",
            "evidence": "spec scenarios: relogin/mount/probe/version fixes are not the grant; S2 forbidden list",
            "input": "host-level Err not grant-fixable (probe, relogin, helper-missing, mount, version)",
            "state": "host-stopped"
          },
          {
            "effect": "no-op",
            "evidence": "connection.rs:227-231 unchanged — regression-protected by last_candidate_failure_reports_one_error (connection.rs:562-608)",
            "input": "start Err(e) non-host-level (config check, spawn, crash budget)",
            "state": "looping"
          },
          {
            "effect": "no-op",
            "evidence": "connection.rs:126-133 Stop path and 317-318 exhausted path unchanged",
            "input": "Stop cmd mid-loop / candidates exhausted",
            "state": "user-stopped / exhausted"
          }
        ]
      },
      "codeTasks": [
        "FIRST — tests in crates/ui/src/connection.rs #[cfg(test)] mod tests (the only test file this chunk adds to): pure grant_fixable rows; stub test host_level_failure_stops_the_candidate_loop — two candidates, xray TUN settings (settings.tun.enabled = true, backend Xray), backend = stub script without caps; assert via next_state()/assert_nothing_after_terminal: one terminal Error whose text contains the first candidate label and not the second and not 'All candidates failed'; TunGrantRequired, when observed, arrives before the terminal Error and carries GENERATION",
        "connection.rs Err(e) arm (~215-231): if e.is_host_level() -> shutdown parked managers, emit AppMsg::TunGrantRequired(generation) when grant_fixable(&e), report(ProcessState::Error(e.to_string()), None), return — else existing push/park/continue",
        "connection.rs: pure fn grant_fixable(e: &ProcessError) -> bool == matches!(e, ProcessError::TunCapabilityMissing(_) | ProcessError::TunHelperCapabilityMissing(_))",
        "app.rs:104-131 AppMsg enum: add TunGrantRequired(u64) — minimal edit so connection.rs compiles; full handling is the S4 chunk"
      ]
    },
    {
      "id": "grant-toast-action",
      "tasks": [
        "3.2",
        "4.1"
      ],
      "summary": "NO-RED-WAIVER: rust-coder writes tests and code in one dispatch. Sites: crates/ui/src/app.rs (+ its inline #[cfg(test)] mod tests — the only test file this chunk adds to); crates/ui/src/preferences/tun.rs for the page name only. Error toast gains a 'Grant TUN privileges' button opening Preferences on the TUN page, gated on generation. NO-TESTER-WAIVER: chunk closes by waiver after guard; verification = the chunk's cargo test command plus the run floor.",
      "contract": {
        "budgets": [
          "toast decision is O(1) pure comparison — no GTK in the predicate; only the toast build touches widgets"
        ],
        "forbidden": [
          "Grant TUN privileges button shown without a matching current-generation TunGrantRequired",
          "button attached to a non-Error toast",
          "grant_generation surviving after the Error toast is shown, or surviving a generation advance",
          "stale-generation TunGrantRequired arming the toast (must be dropped like stale states, app.rs:1189-1195)"
        ],
        "seeding": [
          "Pure unit test only: error_toast_action with literal u64 generations and Option — no GTK init, no component; pattern of app.rs:2034-2053 pure-helper tests",
          "Toast wiring is verified by task 4.2 live manual check (remove helper caps -> connect xray TUN -> toast button opens Preferences on TUN page); no automated UI test — repo has none for toasts"
        ],
        "states": [
          "grant-idle",
          "grant-armed",
          "toast-with-action"
        ],
        "transitions": [
          {
            "effect": "set",
            "evidence": "crates/ui/src/app.rs:1189-1195 generation-gate pattern + is_current_generation app.rs:1515-1517; AppMsg enum app.rs:104-131",
            "input": "AppMsg::TunGrantRequired(g), is_current_generation(g, connection_generation)",
            "state": "grant-armed"
          },
          {
            "effect": "no-op",
            "evidence": "superseded-connection handling precedent app.rs:1190-1195, 1291-1296",
            "input": "AppMsg::TunGrantRequired(g), g != connection_generation",
            "state": "grant-idle"
          },
          {
            "effect": "set",
            "evidence": "task 3.2 + design.md decision 'Grant action on connect'; toast built in the ProcessStateConnection arm where sender is in scope (app.rs:1189), apply_state app.rs:139-146 keeps the plain toast",
            "input": "ProcessState::Error for current generation while grant-armed",
            "state": "toast-with-action"
          },
          {
            "effect": "set",
            "evidence": "task 3.2; button handler -> AppMsg::OpenPreferences (app.rs:1313-1345) + TUN page visible; preferences page built at preferences/mod.rs:75-83 via build_tun_page (preferences/tun.rs:47-50)",
            "input": "user clicks 'Grant TUN privileges'",
            "state": "toast-with-action"
          },
          {
            "effect": "set",
            "evidence": "pure error_toast_action: Some(ToastAction::GrantTun) iff grant_generation == Some(generation), else None — every other Error keeps today's plain toast (app.rs:142-144)",
            "input": "ProcessState::Error with no armed grant or stale generation",
            "state": "grant-idle"
          }
        ]
      },
      "codeTasks": [
        "FIRST — tests in crates/ui/src/app.rs #[cfg(test)] mod tests (pure, pattern app.rs:2034-2053): error_toast_action(g, Some(g)) == Some(ToastAction::GrantTun); error_toast_action(g, Some(g+1)) == None; error_toast_action(g, None) == None — derive Debug+PartialEq on ToastAction",
        "app.rs: enum ToastAction { GrantTun } and pure fn error_toast_action(generation: u64, grant_generation: Option<u64>) -> Option<ToastAction>",
        "app.rs App struct: field grant_generation: Option<u64> (init None, reset when connection_generation advances); AppMsg::TunGrantRequired(generation) arm: if is_current_generation(generation, self.connection_generation) { self.grant_generation = Some(generation) } else ignore",
        "app.rs ProcessStateConnection arm (1189+): on ProcessState::Error build the toast where sender is reachable — plain 'Error: {msg}' toast unchanged when error_toast_action(...) is None; when Some(ToastAction::GrantTun): adw::Toast with button label 'Grant TUN privileges', connect_clicked -> clear grant_generation, emit AppMsg::OpenPreferences and select the TUN page; consume (clear) grant_generation either way",
        "TUN page selection: name the page in crates/ui/src/preferences/tun.rs build_tun_page (tun_page.set_name(Some(\"tun\"))) and present + set_visible_page_name(\"tun\") on the stored/new dialog (app.rs:1313-1345 OpenPreferences flow, preferences/mod.rs:28-83) — adw::PreferencesDialog is already a 1.5 API in this repo, so the page-name API is available"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "### Requirement: TUN requires elevated capabilities granted once The system SHALL require the backend binary to hold `CAP_NET_ADMIN` before a TUN connection starts, and SHALL detect this by reading the binary's file capabilities. For xray, the system SHALL additionally require, before spawning the backend, that the route helper resolves to an existing file this process can execute and that it holds `CAP_NET_ADMIN`. When the application runs with effective user ID 0, capability checks SHALL be skipped. Reading file capabilities SHALL be bounded by a timeout; a timeout or a missing capability-reading tool SHALL fail the start with an error naming the cause. When the backend binary resides on a filesystem that does not honor file capabilities, the start SHALL fail with an error naming the path and the manual `setcap` command. A failure of any of these checks SHALL end the connection attempt without trying further candidates.",
      "tests": [
        "manager::tests::tun_start_refuses_without_capability",
        "privilege::tests::probe_error_text Timeout/NotFound/Exit rows",
        "privilege::tests::caps_check_needed 0/1000 rows",
        "privilege::tests::nosuid_mount_is_unsupported",
        "manager::tests::is_host_level exhaustive variant test",
        "manager::tests helper-gate rows (missing / non-executable / uncapped helper → error before launch, no ` session ` record)",
        "connection::tests host-level stop row (two candidates lacking caps → one start attempt, one terminal Error)",
        "app tests::error_toast_action generation match/mismatch rows"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL NOT start the backend in TUN mode, SHALL NOT try further candidates, and SHALL surface the \"Grant TUN privileges\" action with the error",
      "tests": [
        "manager::tests::tun_start_refuses_without_capability",
        "connection::tests host-level stop row (two candidates lacking caps → one start attempt, one terminal Error)",
        "app tests::error_toast_action generation match/mismatch rows"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL NOT spawn the backend and SHALL surface the \"Grant TUN privileges\" action",
      "tests": [
        "manager::tests helper-gate rows (missing / non-executable / uncapped helper → error before launch, no ` session ` record)",
        "app tests::error_toast_action generation match/mismatch rows"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL NOT spawn the backend and SHALL tell the user to log out and back in to finish enabling TUN",
      "tests": [
        "manager::tests helper-gate rows (missing / non-executable / uncapped helper → error before launch, no ` session ` record) (relogin text row)"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL NOT spawn the backend and SHALL report that the route helper was not found",
      "tests": [
        "manager::tests helper-gate rows (missing / non-executable / uncapped helper → error before launch, no ` session ` record) (missing helper row)"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL NOT read file capabilities and SHALL proceed to start",
      "tests": [
        "privilege::tests::caps_check_needed 0/1000 rows"
      ]
    },
    {
      "shall": "- **THEN** the start SHALL fail without spawning, with an error stating the timeout or naming the missing tool and its package",
      "tests": [
        "privilege::tests::probe_error_text Timeout/NotFound/Exit rows"
      ]
    },
    {
      "shall": "- **THEN** the start SHALL fail without spawning, with an error naming the path and the manual `setcap` command, and SHALL NOT offer the grant action",
      "tests": [
        "manager::tests helper-gate rows (missing / non-executable / uncapped helper → error before launch, no ` session ` record) (nosuid row)",
        "privilege::tests::nosuid_mount_is_unsupported",
        "privilege::tests::mount_lookup_picks_longest_prefix"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL run a single `pkexec` elevation that applies `cap_net_admin,cap_net_bind_service,cap_net_raw+ep` to the backend binary and `cap_net_admin+ep` to the route helper, sets root ownership and the setuid bit on the `v2ray-rs-run` wrapper when it is present, then re-detect capabilities",
      "tests": [
        "privilege::tests::grant_argv_runs_both_setcaps_in_one_elevation",
        "privilege::tests::grant_argv_includes_wrapper_step_when_present"
      ]
    },
    {
      "shall": "- **THEN** the system SHALL detect the missing capability on the next TUN start attempt and re-offer the grant",
      "tests": [
        "manager::tests::tun_start_refuses_without_capability"
      ]
    },
    {
      "shall": "- **THEN** the grant SHALL fail fast before elevation, naming the affected path and pointing at the manual `setcap` command, instead of reporting success while the privileges silently did not take",
      "tests": [
        "privilege::tests::nosuid_mount_is_unsupported",
        "privilege::tests::manual_command_format"
      ]
    }
  ],
  "testHarness": [
    "write_script — crates/process/src/manager.rs — writes executable shell script with mode 0o755",
    "manager_for — crates/process/src/manager.rs — builds ProcessManager with mock script backend and empty config in tempdir",
    "backend_log — crates/process/src/manager.rs — opens Arc<RotatingFileWriter> for backend.log",
    "wait_for_lines — crates/process/src/manager.rs — polls log file until at least n lines appear",
    "stub — crates/ui/src/connection.rs — builds Stub with tempdir, AppPaths, and executable mock backend script",
    "candidate — crates/ui/src/connection.rs — builds ConnectionCandidate for address",
    "connect — crates/ui/src/connection.rs — spawns connection task returning (ConnectionHandle, relm4::Receiver<AppMsg>)",
    "next_state — crates/ui/src/connection.rs — awaits next (ProcessState, Option<ConnectionMetadata>) from receiver with timeout"
  ],
  "floor": "timeout 10m cargo test --workspace -- --test-threads=4 && cargo fmt --check",
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 1
  }
}
```
