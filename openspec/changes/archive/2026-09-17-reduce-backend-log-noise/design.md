## Context

- xray/v2ray connection configs hardcode `"log": { "loglevel": "warning" }` (`crates/core/src/config/v2ray.rs:55`); sing-box hardcodes `"log": { "level": "warn" }` (`crates/core/src/config/singbox.rs:41`). Probe configs set their own `log` objects (`crates/core/src/config/probe.rs:73`, `:101`, `:164`).
- Xray-core documents `log.access` as a file path, unspecified or empty meaning stdout, and the special value `none` disabling the access log; `loglevel` is `debug|info|warning|error|none`, default `warning` (Xray-docs-next `docs/en/config/log.md`). v2ray-core's v4 JSON loader maps `AccessLog == "none"` to a disabled access log (`infra/conf/synthetic/log/log.go`). sing-box's `log` has `disabled`, `level` (`trace|debug|info|warn|error|fatal|panic`), `output`, `timestamp`, and no access-log split (sing-box 1.13.0 `docs/configuration/log/index.md`).
- Backend and helper lines are written by `write_stream_line` → `RotatingFileWriter::append_line`, which prefixes `Utc::now().to_rfc3339()` (`crates/process/src/manager.rs:775-779`, `crates/core/src/rotating_log.rs:92-103`), and pushed unmodified to the buffer and log stream (`manager.rs:557-595`, helper lines `:691-706`). No escape stripping exists anywhere in the workspace.
- Restart detection compares `RuntimeConfigSnapshot` fields with current settings (`crates/core/src/runtime_snapshot.rs:9-34`); the `process-lifecycle` requirement says every setting that changes the generated config is restart-relevant.
- The connection task forwards log lines as `AppMsg::ProcessLogLine` (`crates/ui/src/connection.rs:256-269`); `AppMsg::ShowToast` exists (`crates/ui/src/app.rs:128`).
- The System page holds Interface and Integration groups (`crates/ui/src/preferences/system.rs:11-51`).
- Evidence: rotation set 2026-09-13 08:59 → 2026-09-15 10:11 UTC, 82,190 lines, 68,463 access lines, 58 lines with escapes (all sing-box stderr), 23 xray WebSocket deprecation lines, 8 sing-box `implicit default HTTP client … deprecated` lines, 8 REALITY `potential MITM or redirection` lines.

## Goals / Non-Goals

**Goals:**
- Default `backend.log` carries warnings and records, not traffic; the user can turn traffic lines back on.

**Non-Goals:**
- Rewriting backend timestamps or disabling the writer's UTC prefix.
- Changing rotation size or count.
- Stripping escapes from user-facing failure text (`stop-failover-on-shared-failure`); if that change lands first, reuse its helper instead of adding a second one.
- Recording exit reasons or session DNS fields (`log-connection-decisions`, `surface-effective-dns`).

## Decisions

- **Settings shape.** `AppSettings.logging: LoggingSettings { backend_level: BackendLogLevel, connection_log: bool }`, `#[serde(default)]`, enum serialized lowercase. One section keeps future log knobs together. Added to `RuntimeConfigSnapshot` and `diverges_from`.
- **Access off by default.** Default `connection_log = false` emits `access: "none"`. Alternative rejected: keep access on and filter lines in the app — the backend still formats and pipes every line, and the logs page would still need filtering.
- **sing-box level only.** No split exists; the switch stays insensitive for sing-box with a note rather than silently raising the level. `warning` maps to `warn`.
- **Strip escapes at capture.** One `strip_ansi(&str) -> Cow<str>` in `v2ray-rs-core` (CSI `ESC [ … final byte` and OSC `ESC ] … BEL|ESC \`), applied in `capture_output` and `log_helper` before file, buffer, and stream. Covers every backend and the helper with one call site per stream. Alternative rejected: passing sing-box `--disable-color` — present in the installed 1.14.0 but not verified for every supported version, and it does not cover other emitters.
- **UTC offset, not rewritten clocks.** Session record gains `utc_offset=±HH:MM` from `chrono::Local::now().offset()`. Conservative: backend text stays byte-identical, and one field lets a reader align both clocks.
- **Warning patterns.** A small table `(backend, needle)`: xray/v2ray `is deprecated`, xray `potential MITM or redirection`; sing-box `WARN` + `deprecated`. Matched against escape-stripped lines in the connection task's log forwarder; a per-connection set of matched pattern ids ensures one toast per pattern. Toast text `Backend warning: <line without timestamp prefix, truncated to 160 chars>`. Alternative rejected: toasting every `[Warning]` — xray logs routine dial failures at that level.

## Risks / Trade-offs

- [User debugging traffic finds no access lines] → the note on the switch says where they went; one toggle plus restart restores them.
- [Pattern wording changes upstream] → missed toast only; lines still reach both logs.
- [Toast on every connect for a persistent deprecation] → once per connection, which is the point: the node needs migrating.

## Migration Plan

- Missing `[logging]` loads defaults; the first connect after upgrade drops access lines. Rollback: older builds ignore the unknown section.

## Implementation plan

Tier standard, mode existing-service-strict, base `0e5313d`. Rust stack: every seam carries `NO-RED-WAIVER:` / `NO-TESTER-WAIVER:` — each chunk's coder writes its tests first and the chunk closes by its verify command. Existing tests are read-only except the one amendment named below.

Context correction found while planning: escape stripping already exists as a private CSI-only `strip_ansi` in `crates/ui/src/connection.rs` and `without_ansi` in `crates/process/src/manager.rs`. Per the Non-Goal on reusing the helper, both move into one `v2ray_rs_core::ansi::strip_ansi` and the copies are deleted.

Two lanes run in parallel.

**Lane A — integration worktree**

1. **logging-settings** (1.1, 1.2). `BackendLogLevel { Error, #[default] Warning, Info, Debug }` (`serde(rename_all = "lowercase")`, `as_str`, `singbox_level`), `LoggingSettings { backend_level, connection_log }` (`Copy`, `serde(default)`), `AppSettings.logging` with `#[serde(default)]`, re-exported from `models`. `RuntimeConfigSnapshot.logging` joins `diverges_from` and `restore_settings`; all 8 snapshot literals updated (no `.clone()` on the `Copy` type). Tests: `test_default_logging_settings`, `test_legacy_settings_toml_missing_logging_defaults`, `test_logging_section_missing_key_defaults`, `test_logging_settings_toml_roundtrip_debug_connection_log`, `test_runtime_config_snapshot_detects_logging_divergence`, `test_runtime_config_snapshot_restores_logging`.
2. **generator-log** (2.1, 2.2). Private `log_object(&LoggingSettings)` in `v2ray.rs`: `loglevel` from the level, `access: "none"` unless `connection_log`. sing-box `{"level": singbox_level()}`. Probe configs unchanged. Tests: `xray_log_defaults_silence_access`, `v2ray_log_info_with_connection_log_omits_access`, `v2ray_log_off_sets_access_none_at_every_level`, `singbox_log_maps_levels_and_ignores_connection_log`, `probe_configs_keep_fixed_log_objects`; live-binary cases `log-debug-connection-log` (xray_check) and `log-debug` (singbox_check).
3. **ui-diagnostics** (4.1). `build_system_page` takes `&SettingsObservers`; a Diagnostics group with "Backend log level" combo and "Connection log" switch, tracking the selected backend live. `connection_log_note(BackendType) -> (bool, &'static str)`: sing-box → insensitive, "sing-box writes connection lines at the info and debug levels"; others → "Write accepted connections to the backend log". Tests: `connection_log_note_insensitive_for_singbox_only`, `diagnostics_group_tracks_backend_selection` (through `gtk_test::run`).

**Lane B — shard `hygiene`, rebased onto lane A before the floor**

4. **log-hygiene** (3.1, 3.2). `crates/core/src/ansi.rs`: `pub fn strip_ansi(line: &str) -> Cow<'_, str>`. It removes CSI and OSC sequences (OSC ends at BEL or ESC `\`). A truncated escape at the end of a line drops the rest of the line. A lone ESC is dropped and the next character kept. `rotating_log.rs` gains `format_utc_offset(seconds_east)` and `local_utc_offset()`. Applied in `capture_output` and `log_helper` before file, buffer and stream. The session line becomes `backend=… version=… node=… tun=… utc_offset=±HH:MM <caller fields>`. The one amended existing test is `session_record_appends_caller_fields`, whose assertion becomes `contains("tun=off utc_offset=")` and keeps `ends_with(fields)`. Tests: `strip_ansi_*` (6), `format_utc_offset_signs_hours_minutes`, `backend_escapes_stripped_before_file_buffer_and_stream`, `helper_lines_stripped_of_escapes`, `session_record_states_utc_offset`.
5. **backend-warnings** (4.2, 4.3), after log-hygiene. `crates/ui/src/backend_warning.rs` holds:
   - `PatternId { Deprecated, RealityMitm }`, and a pattern table of `(backend, id, needles)` in which every needle must match, case-sensitive.
   - `match_backend_warning`.
   - `warning_toast`, which builds "Backend warning: " followed by the line with its xray timestamp prefix removed, cut to 159 characters plus `…` when longer than 160.
   - `BackendWarnings`, created once per connect before the `'candidates` loop and shared with every candidate's log forwarder.

   Tests: `matches_xray_websocket_deprecation`, `matches_xray_reality_mitm`, `matches_singbox_warn_deprecation`, `ignores_ordinary_xray_dial_warning`, `ignores_patterns_of_other_backends`, `toast_drops_xray_timestamp`, `toast_truncates_to_160_chars`, `toasts_once_per_pattern`, `reality_warning_toasts_once_per_connection`. The last one collects toasts while waiting for the three log lines too.

**Join**

6. **verification** (5.1, 5.2). Runs the floor after the shard merges. It also adds the CHANGELOG `[Unreleased]` entries and any docs/ARCHITECTURE.md logging mentions. Task 5.2 and the live part of 4.1 are manual checks the operator reports.

Verify per chunk: `timeout 5m cargo test -p <crate> -- --test-threads=4` for the crates it touches, `cargo clippy -p <crates> --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.

Floor: `timeout 10m cargo test --workspace --all-targets -- --test-threads=4 && cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings`.

Lenses: `spec`, `quality`.

Risks:
- The GTK test skips when there is no display.
- A textual merge overlap with `surface-effective-dns` near the session record.
- The access log is now off by default, which is a user-visible change and gets a CHANGELOG entry.

Plan review: zarchitect, 2 rounds, pass. The round 1 blocker was the existing session-record test assertion, resolved by the amendment above.

## Plan appendix

```json
{
  "v": 2,
  "change": "reduce-backend-log-noise",
  "baseSha": "0e5313d8b1588cec759e7f54cf7362931589b5f8",
  "generatedAt": "2026-09-16T19:38:23.977Z",
  "tier": "standard",
  "mode": "existing-service-strict",
  "lenses": [
    "spec",
    "quality"
  ],
  "estimateHours": 3.3,
  "chunks": [
    {
      "id": "logging-settings",
      "taskIds": [
        "1.1",
        "1.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": false,
      "seam": "logging-settings",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "1.1",
          "file": "crates/core/src/models/settings.rs",
          "symbol": "AppSettings.logging / LoggingSettings / BackendLogLevel",
          "anchor": "pub health_check: HealthCheckSettings,",
          "change": "add `#[serde(default)] pub logging: LoggingSettings,` after health_check; define LoggingSettings { backend_level: BackendLogLevel, connection_log: bool } + BackendLogLevel (Error|Warning|Info|Debug, #[serde(rename_all = \"lowercase\")], Default Warning) near HealthCheckSettings (L137-151)"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/models/settings.rs",
          "symbol": "impl Default for AppSettings",
          "anchor": "health_check: HealthCheckSettings::default(),",
          "change": "add `logging: LoggingSettings::default(),`"
        },
        {
          "task": "1.1",
          "file": "crates/core/src/models/settings.rs",
          "symbol": "tests",
          "anchor": "fn test_health_check_settings_round_trip() {",
          "change": "add test_legacy_settings_toml_missing_logging_defaults + test_logging_settings_round_trip (debug/true) following health_check pair L432-451"
        },
        {
          "task": "1.2",
          "file": "crates/core/src/runtime_snapshot.rs",
          "symbol": "RuntimeConfigSnapshot",
          "anchor": "pub ws_heartbeat_secs: u32,",
          "change": "add `pub logging: LoggingSettings,` field; update imports L3-6"
        },
        {
          "task": "1.2",
          "file": "crates/core/src/runtime_snapshot.rs",
          "symbol": "diverges_from",
          "anchor": "|| self.ws_heartbeat_secs != settings.ws_heartbeat_secs",
          "change": "add `|| self.logging != settings.logging`"
        },
        {
          "task": "1.2",
          "file": "crates/core/src/runtime_snapshot.rs",
          "symbol": "restore_settings",
          "anchor": "settings.ws_heartbeat_secs = self.ws_heartbeat_secs;",
          "change": "restore logging (restore_settings mirrors every runtime field)"
        },
        {
          "task": "1.2",
          "file": "crates/core/src/runtime_snapshot.rs",
          "symbol": "tests",
          "anchor": "fn test_runtime_config_snapshot_detects_ws_heartbeat_divergence() {",
          "change": "add logging to make_snapshot (L87-105) and 5 other literal constructors (L159,235,255,339,430); add detects_logging_divergence test (backend_level change)"
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "App connect runtime_snapshot literal",
          "anchor": "ws_heartbeat_secs: self.settings.ws_heartbeat_secs,",
          "change": "add `logging: self.settings.logging.clone(),` (struct literal would not compile otherwise)"
        },
        {
          "task": "1.2",
          "file": "crates/ui/src/app.rs",
          "symbol": "tests::snapshot",
          "anchor": "ws_heartbeat_secs: 30,",
          "change": "add `logging: Default::default(),`"
        }
      ],
      "contract": {
        "states": [
          "level-default",
          "level-set",
          "connlog-off",
          "connlog-on",
          "snapshot-equal",
          "snapshot-diverged"
        ],
        "transitions": [
          {
            "input": "TOML without [logging] section (legacy literal as settings.rs:329)",
            "state": "level-default",
            "effect": "forced",
            "evidence": "app-persistence spec: absent section loads backend_level=warning; design Decisions 'Settings shape' #[serde(default)]"
          },
          {
            "input": "TOML without [logging] section",
            "state": "connlog-off",
            "effect": "forced",
            "evidence": "app-persistence spec scenario 'Legacy settings without the section'"
          },
          {
            "input": "[logging] with only connection_log = true (backend_level key absent)",
            "state": "level-default",
            "effect": "forced",
            "evidence": "app-persistence spec: 'When the section or a key is absent' -> container #[serde(default)] on LoggingSettings"
          },
          {
            "input": "[logging] backend_level = \"debug\"",
            "state": "level-set",
            "effect": "set",
            "evidence": "app-persistence spec scenario 'Round-trip'"
          },
          {
            "input": "[logging] connection_log = true",
            "state": "connlog-on",
            "effect": "set",
            "evidence": "app-persistence spec scenario 'Round-trip'"
          },
          {
            "input": "toml::to_string then toml::from_str of settings with Debug/true",
            "state": "level-set",
            "effect": "no-op",
            "evidence": "round-trip preserves values; settings.rs:305-311 pattern"
          },
          {
            "input": "snapshot built from settings, settings unchanged",
            "state": "snapshot-equal",
            "effect": "no-op",
            "evidence": "runtime_snapshot.rs:28-49"
          },
          {
            "input": "settings.logging.backend_level changed after snapshot",
            "state": "snapshot-diverged",
            "effect": "set",
            "evidence": "tasks 1.2; app-persistence scenario 'Change while connected'"
          },
          {
            "input": "settings.logging.connection_log changed after snapshot",
            "state": "snapshot-diverged",
            "effect": "set",
            "evidence": "app-persistence spec: 'Changing either value while connected SHALL count as restart-relevant'"
          },
          {
            "input": "snapshot.restore_settings(&mut settings)",
            "state": "level-set",
            "effect": "forced",
            "evidence": "runtime_snapshot.rs:51-63 restores every runtime field; logging added alongside tun/timeouts"
          },
          {
            "input": "unknown level literal e.g. backend_level = \"trace\"",
            "state": "level-default",
            "effect": "no-op",
            "evidence": "out of spec; serde returns Err like other lowercase enums (Language). No test asserts it; do not add a lenient deserializer"
          }
        ],
        "forbidden": [
          "backend_level serialized as anything other than one of: error, warning, info, debug",
          "RuntimeConfigSnapshot without a logging field while diverges_from ignores settings.logging",
          "LoggingSettings::default() other than { backend_level: Warning, connection_log: false }"
        ],
        "seeding": [
          "level-default/connlog-off: toml::from_str::<AppSettings>(legacy literal copied from settings.rs:329 test) or AppSettings::default()",
          "level-set/connlog-on: AppSettings { logging: LoggingSettings { backend_level: BackendLogLevel::Debug, connection_log: true }, ..AppSettings::default() } (plain data, the persisted shape)",
          "snapshot-equal: make_snapshot(BackendType::Xray, \"/usr/bin/xray\") (runtime_snapshot.rs:87) whose logging is LoggingSettings::default(), vs settings with matching backend (pattern runtime_snapshot.rs:405-420)",
          "snapshot-diverged: mutate settings.logging.backend_level / settings.logging.connection_log after the snapshot, never mutate the snapshot"
        ],
        "budgets": [
          "no new dependency",
          "every existing RuntimeConfigSnapshot literal updated: runtime_snapshot.rs:88,159,235,255,339 and app.rs:616,2231"
        ],
        "names": {
          "enum": "pub enum BackendLogLevel { Error, #[default] Warning, Info, Debug } with #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = \"lowercase\")]",
          "enum_methods": "impl BackendLogLevel { pub const ALL: [BackendLogLevel; 4] = [Error, Warning, Info, Debug]; pub const fn as_str(self) -> &'static str /* error|warning|info|debug */; pub const fn singbox_level(self) -> &'static str /* error|warn|info|debug */ }",
          "struct": "pub struct LoggingSettings { pub backend_level: BackendLogLevel, pub connection_log: bool } with #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(default)]",
          "field": "AppSettings: #[serde(default)] pub logging: LoggingSettings, placed after health_check; AppSettings::default() sets logging: LoggingSettings::default()",
          "reexport": "crates/core/src/models/mod.rs:28 pub use settings::{..., BackendLogLevel, LoggingSettings, ...}",
          "snapshot_field": "RuntimeConfigSnapshot { pub logging: LoggingSettings }; diverges_from adds `|| self.logging != settings.logging`; restore_settings adds `settings.logging = self.logging;`; app.rs:616 adds `logging: self.settings.logging`",
          "tests_1.1": "settings.rs tests: test_default_logging_settings, test_legacy_settings_toml_missing_logging_defaults, test_logging_section_missing_key_defaults, test_logging_settings_toml_roundtrip_debug_connection_log",
          "tests_1.2": "runtime_snapshot.rs tests: test_runtime_config_snapshot_detects_logging_divergence, test_runtime_config_snapshot_restores_logging"
        }
      },
      "redTasks": [],
      "codeTasks": [
        "1.1",
        "1.2"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command.",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4 && cargo clippy -p v2ray-rs-core -p v2ray-rs-ui --all-targets --all-features -- -D warnings && cargo fmt --all -- --check",
      "notes": [
        "app.rs snapshot literals use logging: self.settings.logging (Copy, no .clone())",
        "cover all 8 RuntimeConfigSnapshot literals incl. runtime_snapshot.rs tests and app.rs tests::snapshot"
      ]
    },
    {
      "id": "generator-log",
      "taskIds": [
        "2.1",
        "2.2"
      ],
      "prev": "logging-settings",
      "sharedPkg": "crates/core",
      "parallel": false,
      "seam": "generator-log",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "2.1",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "generate_v2ray_family_config",
          "anchor": "\"log\": { \"loglevel\": \"warning\" },",
          "change": "log from settings.logging: loglevel string + `access: \"none\"` unless connection_log; shared by V2ray and Xray (xray.rs calls this fn, does not touch log)"
        },
        {
          "task": "2.1",
          "file": "crates/core/src/config/v2ray.rs",
          "symbol": "tests",
          "anchor": "fn test_policy_conn_idle_from_settings() {",
          "change": "add log tests iterating both V2rayFamilyBackend variants (same shape as policy test)"
        },
        {
          "task": "2.1",
          "file": "crates/core/src/config/singbox.rs",
          "symbol": "assemble",
          "anchor": "\"log\": { \"level\": \"warn\" },",
          "change": "level from settings.logging.backend_level, Warning→\"warn\"; no access key"
        },
        {
          "task": "2.1",
          "file": "crates/core/src/config/singbox.rs",
          "symbol": "tests",
          "anchor": "fn test_singbox_basic_structure() {",
          "change": "add level mapping test"
        },
        {
          "task": "2.2",
          "file": "crates/core/tests/xray_check.rs",
          "symbol": "generated_xray_configs_pass_xray_test",
          "anchor": "fn generated_xray_configs_pass_xray_test() {",
          "change": "add a case with logging debug + connection_log true to `cases`"
        },
        {
          "task": "2.2",
          "file": "crates/core/tests/singbox_check.rs",
          "symbol": "generated_singbox_configs_pass_sing_box_check",
          "anchor": "fn generated_singbox_configs_pass_sing_box_check() {",
          "change": "add debug-level case to `cases`"
        }
      ],
      "contract": {
        "states": [
          "v2ray-log",
          "xray-log",
          "singbox-log",
          "probe-log"
        ],
        "transitions": [
          {
            "input": "XrayGenerator.generate with AppSettings::default()",
            "state": "xray-log",
            "effect": "set",
            "evidence": "config-generator scenario 'Defaults silence xray access lines': log == {\"loglevel\":\"warning\",\"access\":\"none\"}"
          },
          {
            "input": "V2rayGenerator.generate with level Info, connection_log true",
            "state": "v2ray-log",
            "effect": "set",
            "evidence": "config-generator scenario 'Connection log on for v2ray': loglevel info, access key absent (not empty string, not null)"
          },
          {
            "input": "V2ray/Xray generate with connection_log false, any level",
            "state": "v2ray-log",
            "effect": "set",
            "evidence": "config-generator requirement: access SHALL be \"none\" unless connection log on"
          },
          {
            "input": "SingboxGenerator.generate with level Warning, connection_log true",
            "state": "singbox-log",
            "effect": "set",
            "evidence": "config-generator scenario 'sing-box level mapping': log == {\"level\":\"warn\"} exactly"
          },
          {
            "input": "SingboxGenerator.generate with each BackendLogLevel, connection_log false and true",
            "state": "singbox-log",
            "effect": "set",
            "evidence": "mapping error->error, warning->warn, info->info, debug->debug; connection_log no effect"
          },
          {
            "input": "probe_generator_for(backend).generate(...) for all three backends",
            "state": "probe-log",
            "effect": "no-op",
            "evidence": "probe.rs:73 {\"level\":\"warn\"}, :101 and :165 {\"loglevel\":\"warning\"}; ProbeConfigGenerator::generate takes no AppSettings (probe.rs:26-32)"
          },
          {
            "input": "xray run -test / sing-box check on generated config with level Debug + connection_log true",
            "state": "xray-log",
            "effect": "no-op",
            "evidence": "tasks 2.2; xray_check.rs:98-170, singbox_check.rs:193+"
          }
        ],
        "forbidden": [
          "v2ray/xray log containing \"access\" when connection_log is true",
          "v2ray/xray log without \"access\":\"none\" when connection_log is false",
          "sing-box log containing any key other than \"level\" (no timestamp/output/disabled)",
          "sing-box level literal \"warning\"",
          "probe generators reading AppSettings.logging"
        ],
        "seeding": [
          "settings via AppSettings { logging: LoggingSettings { .. }, ..default_settings() } in each generator's test module (v2ray.rs default_settings()/vless_node() at :1010, xray.rs tests, singbox.rs tests)",
          "live checks: new case pushed into existing cases vecs, not a new test fn"
        ],
        "budgets": [
          "log object exactly 2 keys (xray/v2ray off), 1 key (xray/v2ray on), 1 key (sing-box)",
          "integration check run under timeout 5m"
        ],
        "names": {
          "helper": "fn log_object(logging: &LoggingSettings) -> Value in crates/core/src/config/v2ray.rs (private); sing-box inline json!({ \"level\": settings.logging.backend_level.singbox_level() })",
          "tests_2.1": "xray.rs: xray_log_defaults_silence_access; v2ray.rs: v2ray_log_info_with_connection_log_omits_access, v2ray_log_off_sets_access_none_at_every_level; singbox.rs: singbox_log_maps_levels_and_ignores_connection_log; probe.rs: probe_configs_keep_fixed_log_objects",
          "tests_2.2": "xray_check.rs generated_xray_configs_pass_xray_test gains case \"log-debug-connection-log\"; singbox_check.rs generated_singbox_configs_pass_sing_box_check gains case \"log-debug\" (level Debug, connection_log true)"
        }
      },
      "redTasks": [],
      "codeTasks": [
        "2.1",
        "2.2"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command.",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4 && cargo clippy -p v2ray-rs-core --all-targets --all-features -- -D warnings && cargo fmt --all -- --check"
    },
    {
      "id": "ui-diagnostics",
      "taskIds": [
        "4.1"
      ],
      "prev": "generator-log",
      "sharedPkg": "crates/ui",
      "parallel": false,
      "seam": "ui-diagnostics",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "4.1",
          "file": "crates/ui/src/preferences/system.rs",
          "symbol": "build_system_page",
          "anchor": "page.add(&integration_group);",
          "change": "add Diagnostics PreferencesGroup: ComboRow level + SwitchRow connection log; switch sensitivity/note by state.borrow().backend.backend_type"
        },
        {
          "task": "4.1",
          "file": "crates/ui/src/preferences/system.rs",
          "symbol": "handlers",
          "anchor": "notif_row.connect_active_notify(move |row| {",
          "change": "add connect_selected_notify / connect_active_notify handlers mutating st.borrow_mut().logging then emit(&st, &cb)"
        }
      ],
      "contract": {
        "states": [
          "level-shown",
          "switch-sensitive",
          "switch-insensitive"
        ],
        "transitions": [
          {
            "input": "page built with xray (AppSettings::default())",
            "state": "level-shown",
            "effect": "set",
            "evidence": "diagnostic-logs scenario 'Controls shown': combo selected index 1 ('warning'), switch inactive"
          },
          {
            "input": "page built with xray or v2ray",
            "state": "switch-sensitive",
            "effect": "set",
            "evidence": "scenario 'Controls shown': visible and sensitive; subtitle CONNECTION_LOG_NOTE"
          },
          {
            "input": "page built with sing-box",
            "state": "switch-insensitive",
            "effect": "set",
            "evidence": "scenario 'sing-box note'; subtitle SINGBOX_CONNECTION_LOG_NOTE"
          },
          {
            "input": "settings observer fired with backend changed to sing-box (Network page selection)",
            "state": "switch-insensitive",
            "effect": "set",
            "evidence": "mod.rs:48-59 fan-out; dns.rs:499-501 live-backend pattern"
          },
          {
            "input": "settings observer fired with backend changed back to xray/v2ray",
            "state": "switch-sensitive",
            "effect": "set",
            "evidence": "same fan-out"
          },
          {
            "input": "combo selection changed",
            "state": "level-shown",
            "effect": "set",
            "evidence": "writes state.logging.backend_level = BackendLogLevel::ALL[selected], then emit"
          },
          {
            "input": "switch toggled (only reachable while sensitive)",
            "state": "switch-sensitive",
            "effect": "no-op",
            "evidence": "writes state.logging.connection_log, then emit; sensitivity unchanged"
          },
          {
            "input": "sing-box selected while connection_log persisted true",
            "state": "switch-insensitive",
            "effect": "no-op",
            "evidence": "switch keeps active=true, insensitive; generator ignores it for sing-box; value not rewritten"
          }
        ],
        "forbidden": [
          "switch sensitive while settings.backend.backend_type == SingBox",
          "writing connection_log=false when the backend becomes sing-box",
          "observer callback holding state.borrow() across emit (mod.rs:97-99 invariant)"
        ],
        "seeding": [
          "pure: connection_log_note(BackendType::SingBox / Xray / V2ray)",
          "GTK: crate::gtk_test::run (gtk_test.rs:13) building the group via build_diagnostics_group with Rc<RefCell<AppSettings>>, a no-op SettingsCallback and a SettingsObservers vec; backend change seeded by invoking the observers with a settings clone whose backend_type is SingBox (the same path emit uses), never by calling set_sensitive in the test; skips with 'no display, skipping' when headless"
        ],
        "budgets": [
          "no new crate deps",
          "one observer registration"
        ],
        "names": {
          "signature": "pub(super) fn build_system_page(state: &Rc<RefCell<AppSettings>>, cb: &SettingsCallback, observers: &SettingsObservers) -> adw::PreferencesPage; mod.rs:63 passes &settings_observers",
          "group_builder": "fn build_diagnostics_group(state: &Rc<RefCell<AppSettings>>, cb: &SettingsCallback, observers: &SettingsObservers) -> (adw::PreferencesGroup, adw::ComboRow, adw::SwitchRow)",
          "group_title": "Diagnostics",
          "combo_title": "Backend log level",
          "switch_title": "Connection log",
          "note_fn": "fn connection_log_note(backend: BackendType) -> (bool, &'static str) /* (sensitive, subtitle) */",
          "notes": "const CONNECTION_LOG_NOTE: &str = \"Write accepted connections to the backend log\"; const SINGBOX_CONNECTION_LOG_NOTE: &str = \"sing-box writes connection lines at the info and debug levels\"",
          "tests_4.1": "system.rs: connection_log_note_insensitive_for_singbox_only, diagnostics_group_tracks_backend_selection (gtk_test::run)"
        }
      },
      "redTasks": [],
      "codeTasks": [
        "4.1"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command.",
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings && cargo fmt --all -- --check",
      "notes": [
        "build_system_page gains observers; preferences/mod.rs build call passes &settings_observers",
        "GTK tests go through crate::gtk_test::run"
      ]
    },
    {
      "id": "log-hygiene",
      "taskIds": [
        "3.1",
        "3.2"
      ],
      "prev": null,
      "sharedPkg": null,
      "parallel": true,
      "seam": "log-hygiene",
      "shard": "hygiene",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "3.1",
          "file": "crates/core/src/lib.rs",
          "symbol": "mod list",
          "anchor": "pub mod rotating_log;",
          "change": "declare new module (e.g. `pub mod ansi;`) holding `strip_ansi(&str) -> Cow<str>` CSI+OSC with unit tests"
        },
        {
          "task": "3.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "capture_output stdout",
          "anchor": "write_stream_line(&writer, \"stdout\", &line);",
          "change": "strip before write/LogLine::stdout/tx.send/buffer push"
        },
        {
          "task": "3.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "capture_output stderr",
          "anchor": "write_stream_line(&writer, \"stderr\", &line);",
          "change": "same for stderr"
        },
        {
          "task": "3.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "log_helper",
          "anchor": "write_stream_line(&self.log_writer, \"helper\", &content);",
          "change": "strip content before write, LogLine::stderr, emit"
        },
        {
          "task": "3.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "write_session_record",
          "anchor": "let mut record = format!(\"backend={backend} version={version} node={node} tun={tun}\");",
          "change": "session line gains utc_offset=±HH:MM after tun= and before the caller fields, from v2ray_rs_core::rotating_log::local_utc_offset(); no chrono dependency in process"
        },
        {
          "task": "3.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "without_ansi",
          "anchor": "fn without_ansi(line: &str) -> String {",
          "change": "existing CSI-only helper used by last_error_line; candidate for replacement by core strip_ansi"
        },
        {
          "task": "3.2",
          "file": "crates/process/src/manager.rs",
          "symbol": "tests",
          "anchor": "async fn session_record_precedes_spawn() {",
          "change": "add stub test printing escapes → backend.log has no \\x1b, session line contains utc_offset="
        },
        {
          "task": "3.1",
          "file": "crates/core/src/ansi.rs",
          "symbol": "strip_ansi",
          "anchor": "",
          "change": "new file: pub fn strip_ansi(line: &str) -> Cow<'_, str> (CSI + OSC) with unit tests",
          "new": true
        }
      ],
      "contract": {
        "states": [
          "borrowed",
          "stripped",
          "file-clean",
          "buffer-clean",
          "stream-clean",
          "offset-recorded"
        ],
        "transitions": [
          {
            "input": "line with no U+001B",
            "state": "borrowed",
            "effect": "no-op",
            "evidence": "tasks 3.1 'plain line unchanged and borrowed'; return Cow::Borrowed"
          },
          {
            "input": "CSI: ESC '[' then 0x30-0x3F* then 0x20-0x2F* then one final 0x40-0x7E",
            "state": "stripped",
            "effect": "clear",
            "evidence": "design 'Strip escapes at capture'; existing connection.rs:1035-1046 grammar. Whole sequence removed"
          },
          {
            "input": "CSI whose next char after params/intermediates is not a final byte (e.g. ESC [ 3 \\u{80})",
            "state": "stripped",
            "effect": "clear",
            "evidence": "connection.rs:1046 next_if: consumed prefix removed, the offending char kept"
          },
          {
            "input": "truncated CSI at end of line (\"WARN\\u{1b}[3\" or \"x\\u{1b}[\")",
            "state": "stripped",
            "effect": "clear",
            "evidence": "tasks 3.1 'truncated escape at end of line' -> \"WARN\" / \"x\""
          },
          {
            "input": "OSC: ESC ']' ... terminated by BEL 0x07 or ST (ESC backslash)",
            "state": "stripped",
            "effect": "clear",
            "evidence": "design: OSC ESC ] ... BEL|ESC backslash; terminator removed with the body"
          },
          {
            "input": "unterminated OSC to end of line",
            "state": "stripped",
            "effect": "clear",
            "evidence": "same rule as truncated CSI: removed through end of line"
          },
          {
            "input": "lone ESC followed by any other char, or ESC as last char",
            "state": "stripped",
            "effect": "clear",
            "evidence": "connection.rs:1035-1037: ESC dropped, following char kept"
          },
          {
            "input": "sing-box stderr \"\\u{1b}[31mFATAL\\u{1b}[0m[0000] start service: boom\" via stub backend",
            "state": "file-clean",
            "effect": "forced",
            "evidence": "diagnostic-logs scenario 'sing-box color codes'; backend.log line contains \"stderr FATAL[0000] start service: boom\" and no 0x1b byte anywhere in the file"
          },
          {
            "input": "same stub line",
            "state": "buffer-clean",
            "effect": "forced",
            "evidence": "design: before file, buffer, and stream; LogBuffer content == \"FATAL[0000] start service: boom\""
          },
          {
            "input": "same stub line",
            "state": "stream-clean",
            "effect": "forced",
            "evidence": "ProcessEvent::LogLine from subscribe_logs() carries stripped content"
          },
          {
            "input": "HelperRun { output: [\"\\u{1b}[1mlink up\\u{1b}[0m\"], result: Err(\"\\u{1b}[31mno\\u{1b}[0m\") }",
            "state": "file-clean",
            "effect": "forced",
            "evidence": "manager.rs:1013-1028; both output lines and the '{verb} failed: {e}' line are stripped before write/buffer/emit"
          },
          {
            "input": "launch() with log writer attached",
            "state": "offset-recorded",
            "effect": "set",
            "evidence": "manager.rs:681-702; record = \"backend={backend} version={version} node={node} tun={tun} utc_offset={offset}\" then optional ' ' + session_fields"
          },
          {
            "input": "format_utc_offset(10800) / (0) / (-19800) / (20700)",
            "state": "offset-recorded",
            "effect": "set",
            "evidence": "\"+03:00\" / \"+00:00\" / \"-05:30\" / \"+05:45\"; seconds component dropped; diagnostic-logs scenario 'UTC offset recorded'"
          },
          {
            "input": "backend timestamp text inside the line (xray '2026/09/14 09:57:30.532567 ')",
            "state": "file-clean",
            "effect": "no-op",
            "evidence": "design 'UTC offset, not rewritten clocks': backend text byte-identical apart from escapes"
          }
        ],
        "forbidden": [
          "any U+001B in the return value of strip_ansi",
          "Cow::Owned returned for an input without U+001B",
          "an escape byte in backend.log, LogBuffer or broadcast content originating from capture_output or log_helper",
          "stripping applied separately per sink (must be one call per line, result reused for file, buffer, stream)",
          "rewriting or removing the backend's own timestamp, or changing RotatingFileWriter's RFC 3339 prefix",
          "utc_offset read from Utc or computed per stdout line; it is one field per session record",
          "keeping connection.rs strip_ansi or manager.rs without_ansi alongside the core one"
        ],
        "seeding": [
          "borrowed/stripped: direct calls to v2ray_rs_core::ansi::strip_ansi with literals",
          "file-clean/buffer-clean/stream-clean: manager_for(&dir, script) (manager.rs:1185) .with_log_file(Some(backend_log(dir.path()))), subscribe_logs() before start, set_auto_restart(false), start_with_connection(None), wait_and_handle_exit(); script emits via printf '\\033[31mFATAL\\033[0m[0000] start service: boom\\n' >&2 and printf '\\033]0;t\\007plain out\\n'; never push raw lines into LogBuffer directly",
          "helper path: call mgr.log_helper(\"xray-up\", &HelperRun { output, result, timed_out: false }) from the manager.rs test module on a manager with a log file",
          "offset-recorded: same stub start; assert session line contains format!(\"utc_offset={}\", local_utc_offset()) and matches /utc_offset=[+-]\\d{2}:\\d{2}/; the +03:00 scenario is covered by format_utc_offset(10800) (TZ is process-global, never set TZ in tests)"
        ],
        "budgets": [
          "strip_ansi single pass O(len), zero allocation when no ESC",
          "one strip_ansi call per captured line",
          "process tests under timeout 5m, stub scripts exit within 1s"
        ],
        "names": {
          "module": "crates/core/src/ansi.rs; `pub mod ansi;` in crates/core/src/lib.rs",
          "fn": "pub fn strip_ansi(line: &str) -> Cow<'_, str>",
          "offset_fns": "crates/core/src/rotating_log.rs: pub fn format_utc_offset(seconds_east: i32) -> String; pub fn local_utc_offset() -> String { format_utc_offset(chrono::Local::now().offset().local_minus_utc()) }",
          "session_field": "utc_offset=",
          "removed": "crates/ui/src/connection.rs fn strip_ansi (1026-1049), crates/process/src/manager.rs fn without_ansi (1110-1129)",
          "tests_3.1": "ansi.rs: strip_ansi_removes_singbox_color_codes, strip_ansi_plain_line_is_borrowed, strip_ansi_truncated_csi_at_end_drops_tail, strip_ansi_removes_osc_terminated_by_bel_and_st, strip_ansi_unterminated_osc_drops_tail, strip_ansi_drops_lone_escape; rotating_log.rs: format_utc_offset_signs_hours_minutes",
          "tests_3.2": "manager.rs: backend_escapes_stripped_before_file_buffer_and_stream, helper_lines_stripped_of_escapes, session_record_states_utc_offset"
        }
      },
      "redTasks": [],
      "codeTasks": [
        "3.1",
        "3.2"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command.",
      "verify": "timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4 && timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4 && timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-core -p v2ray-rs-process -p v2ray-rs-ui --all-targets --all-features -- -D warnings && cargo fmt --all -- --check",
      "notes": [
        "delete crates/ui/src/connection.rs strip_ansi and crates/process/src/manager.rs without_ansi, callers use v2ray_rs_core::ansi::strip_ansi",
        "existing tests last_error_matches_through_ansi, failure_key_ignores_xray_timestamps and FATAL summary tests stay green",
        "session line format: backend=.. version=.. node=.. tun=.. utc_offset=±HH:MM then the caller fields; amend existing test session_record_appends_caller_fields (manager.rs) assertion contains(\"tun=off hijack=hijack\") to contains(\"tun=off utc_offset=\") and keep its ends_with(fields) — the only sanctioned edit to an existing test",
        "connection.rs caller storing strip_ansi result into CandidateFailure.reason: String uses .into_owned()",
        "core strip_ansi rule for ESC not followed by [ or ]: drop the ESC and keep the next char; confirm last_error_matches_through_ansi stays green"
      ]
    },
    {
      "id": "backend-warnings",
      "taskIds": [
        "4.2",
        "4.3"
      ],
      "prev": "log-hygiene",
      "sharedPkg": "crates/ui",
      "parallel": false,
      "seam": "backend-warnings",
      "shard": "hygiene",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [
        {
          "task": "4.2",
          "file": "crates/ui/src/connection.rs",
          "symbol": "match_backend_warning / PatternId",
          "anchor": "fn strip_leading_timestamp(text: &str) -> &str {",
          "change": "pure fn + pattern table + unit tests; module placement options in patterns"
        },
        {
          "task": "4.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "log_forwarder",
          "anchor": "log_sender.emit(AppMsg::ProcessLogLine(generation, line.content));",
          "change": "match line, emit AppMsg::ShowToast once per PatternId per connection"
        },
        {
          "task": "4.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "per-connection state",
          "anchor": "let mut capture_notice_sent = false;",
          "change": "declare once-per-connection seen-pattern set before candidate loop (forwarder is spawned per candidate inside the loop, so set must be shared into the spawned task)"
        },
        {
          "task": "4.3",
          "file": "crates/ui/src/connection.rs",
          "symbol": "tests",
          "anchor": "async fn drain_within(",
          "change": "stub-backend test: three REALITY lines → toasts.len()==1 (drain_within already collects toasts)"
        },
        {
          "task": "4.2",
          "file": "crates/ui/src/backend_warning.rs",
          "symbol": "PatternId / match_backend_warning / warning_toast / BackendWarnings",
          "anchor": "",
          "change": "new file: per seam contract names; declared `mod backend_warning;` in crates/ui/src/lib.rs",
          "new": true
        }
      ],
      "contract": {
        "states": [
          "no-match",
          "matched-deprecated",
          "matched-reality",
          "toasted",
          "suppressed"
        ],
        "transitions": [
          {
            "input": "(Xray, \"2026/09/15 08:51:54.138024 [Warning] common/errors: The feature WebSocket transport (with ALPN http/1.1, etc.) is deprecated, not recommended for using and might be removed. Please migrate to XHTTP H2 & H3 as soon as possible.\")",
            "state": "matched-deprecated",
            "effect": "set",
            "evidence": "diagnostic-logs scenario 'xray WebSocket deprecation'; recorded format in backend.log 2026-09-15"
          },
          {
            "input": "(V2ray, any line containing \"is deprecated\")",
            "state": "matched-deprecated",
            "effect": "set",
            "evidence": "design Warning patterns: xray/v2ray `is deprecated`"
          },
          {
            "input": "(Xray, \"2026/09/14 09:57:30.532567 [Error] [1944052120] transport/internet/reality: REALITY: received real certificate (potential MITM or redirection)\")",
            "state": "matched-reality",
            "effect": "set",
            "evidence": "spec REALITY pattern; recorded backend.log 2026-09-14"
          },
          {
            "input": "(SingBox, \"WARN[0000] implicit default HTTP client using default outbound for remote rule-sets is deprecated in sing-box 1.14.0 and will be removed in sing-box 1.16.0.\")",
            "state": "matched-deprecated",
            "effect": "set",
            "evidence": "spec sing-box WARN deprecated; recorded stripped form (sing-box writes no timestamp when piped)"
          },
          {
            "input": "(Xray, \"2026/09/14 09:57:31.000000 [Warning] [123] app/dispatcher: failed to dial tcp 203.0.113.1:443 > dial tcp: i/o timeout\")",
            "state": "no-match",
            "effect": "no-op",
            "evidence": "tasks 4.2 ordinary [Warning] dial failure; design rejects toasting every [Warning]"
          },
          {
            "input": "(Xray, \"[Warning] core: Xray 26.9.9 started\")",
            "state": "no-match",
            "effect": "no-op",
            "evidence": "recorded startup line"
          },
          {
            "input": "(SingBox, \"INFO[0000] ... deprecated ...\") or (SingBox, WebSocket xray line)",
            "state": "no-match",
            "effect": "no-op",
            "evidence": "sing-box needs both WARN and deprecated; xray-only needles not applied to sing-box"
          },
          {
            "input": "(V2ray, REALITY MITM line)",
            "state": "no-match",
            "effect": "no-op",
            "evidence": "design: MITM needle is xray only (v2ray refuses REALITY nodes, bcf47b2/05f4e90)"
          },
          {
            "input": "BackendWarnings::toast_for on first line matching a PatternId",
            "state": "toasted",
            "effect": "set",
            "evidence": "spec: first matching line toasts, quoting it"
          },
          {
            "input": "toast_for on a later line with an already-seen PatternId (same connection)",
            "state": "suppressed",
            "effect": "no-op",
            "evidence": "scenario 'Repeated REALITY warning': exactly one toast"
          },
          {
            "input": "toast_for on a line of a different PatternId after one toasted",
            "state": "toasted",
            "effect": "set",
            "evidence": "at most once per pattern, not per connection"
          },
          {
            "input": "failover to next candidate or in-place respawn within the same Connect",
            "state": "suppressed",
            "effect": "no-op",
            "evidence": "shared Arc set outside the candidate loop, like capture_notice_sent connection.rs:209-211"
          },
          {
            "input": "new Connect (new spawn_with task / generation)",
            "state": "no-match",
            "effect": "clear",
            "evidence": "set owned by the task; design 'once per connection'"
          },
          {
            "input": "stub xray connection echoing the REALITY line three times",
            "state": "toasted",
            "effect": "forced",
            "evidence": "tasks 4.3: toasts == [expected] exactly once; all three lines still arrive as ProcessLogLine (scenario 'Warnings are never access noise')"
          }
        ],
        "forbidden": [
          "more than one ShowToast per PatternId per connection",
          "suppressing or reordering the ProcessLogLine for a matched line",
          "toast quote longer than 160 chars",
          "per-candidate or per-forwarder set (would re-toast on failover)",
          "matching against raw lines with escapes stripped a second time in the ui (process layer owns stripping)"
        ],
        "seeding": [
          "matcher: direct match_backend_warning(BackendType, literal) calls with the recorded lines above",
          "once-set: BackendWarnings::new(BackendType::Xray) then toast_for x3 -> Some, None, None",
          "4.3: stub(r#\"[ \"$1\" = version ] && echo \"Xray 26.6.27\" && exit 0; [ \"$2\" = -test ] && exit 0; for i in 1 2 3; do echo \"2026/09/14 09:57:30.532567 [Error] [1944052120] transport/internet/reality: REALITY: received real certificate (potential MITM or redirection)\"; done; exec sleep 30\"#); settings = AppSettings default with backend Xray and socks_port bound to a live TcpListener (xray analog of ready_singbox_settings :1320, tun off); connect(&stub, settings, vec![candidate(\"203.0.113.1\")]); wait until three ProcessLogLine containing \"potential MITM\" were received, handle.stop(StopReason::UserStop), then drain_within collects the rest"
        ],
        "budgets": [
          "quote <= 160 chars (chars, not bytes)",
          "<= 1 toast per PatternId per connection",
          "4.3 test bounded by RECV_TIMEOUT 20s per message and timeout 5m overall"
        ],
        "names": {
          "module": "crates/ui/src/backend_warning.rs; `mod backend_warning;` in crates/ui/src/lib.rs",
          "pattern_id": "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] pub(crate) enum PatternId { Deprecated, RealityMitm }",
          "table": "const PATTERNS: &[(BackendType, PatternId, &[&str])] = &[(BackendType::Xray, PatternId::Deprecated, &[\"is deprecated\"]), (BackendType::V2ray, PatternId::Deprecated, &[\"is deprecated\"]), (BackendType::Xray, PatternId::RealityMitm, &[\"potential MITM or redirection\"]), (BackendType::SingBox, PatternId::Deprecated, &[\"WARN\", \"deprecated\"])]; first row whose backend equals and whose needles are all contained (case-sensitive) wins",
          "matcher": "pub(crate) fn match_backend_warning(backend: BackendType, line: &str) -> Option<PatternId>",
          "toast_fn": "pub(crate) fn warning_toast(line: &str) -> String = format!(\"Backend warning: {quote}\") where quote = crate::connection::strip_leading_timestamp(line.trim()) (made pub(crate); removes one leading YYYY/MM/DD HH:MM:SS[.frac] plus following spaces, the xray/v2ray form; sing-box lines carry no timestamp and are quoted as-is incl. WARN[0000]); if quote.chars().count() > 160 then first 159 chars + '…' (exactly 160 chars)",
          "set": "pub(crate) struct BackendWarnings { backend: BackendType, seen: HashSet<PatternId> }; pub(crate) fn new(backend: BackendType) -> Self; pub(crate) fn toast_for(&mut self, line: &str) -> Option<String>",
          "forwarder_binding": "let warnings = Arc::new(std::sync::Mutex::new(BackendWarnings::new(settings.backend.backend_type))); before 'candidates loop; Arc::clone into log_forwarder",
          "expected_reality_toast": "Backend warning: [Error] [1944052120] transport/internet/reality: REALITY: received real certificate (potential MITM or redirection)",
          "tests_4.2": "backend_warning.rs: matches_xray_websocket_deprecation, matches_xray_reality_mitm, matches_singbox_warn_deprecation, ignores_ordinary_xray_dial_warning, ignores_patterns_of_other_backends, toast_drops_xray_timestamp, toast_truncates_to_160_chars, toasts_once_per_pattern",
          "tests_4.3": "connection.rs: reality_warning_toasts_once_per_connection"
        }
      },
      "redTasks": [],
      "codeTasks": [
        "4.2",
        "4.3"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "rust-coder",
      "waiver": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command.",
      "verify": "timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4 && cargo clippy -p v2ray-rs-ui --all-targets --all-features -- -D warnings && cargo fmt --all -- --check",
      "notes": [
        "4.3 test: collect ShowToast messages seen while waiting for the three ProcessLogLine lines as well as in drain_within, then assert exactly one toast"
      ]
    },
    {
      "id": "verification",
      "taskIds": [
        "5.1",
        "5.2"
      ],
      "prev": "ui-diagnostics",
      "sharedPkg": "workspace",
      "parallel": false,
      "seam": "verification",
      "shard": "",
      "pkgDirs": [],
      "pkgs": [],
      "sites": [],
      "contract": {
        "states": [
          "floor-green",
          "live-verified"
        ],
        "transitions": [
          {
            "input": "fullFloor command",
            "state": "floor-green",
            "effect": "set",
            "evidence": "tasks 5.1"
          },
          {
            "input": "manual live session per 5.2 and 4.1 live check",
            "state": "live-verified",
            "effect": "set",
            "evidence": "tasks 5.2, 4.1; manual"
          }
        ],
        "forbidden": [
          "marking 5.2 or the live part of 4.1 done from automated runs alone"
        ],
        "seeding": [
          "live: installed /usr/bin/xray and /usr/bin/sing-box with a real ws node; counting new lines only after the latest session record"
        ],
        "budgets": [
          "5.2 live window 10 minutes",
          "floor under timeout 10m"
        ],
        "names": {
          "live_checks": "no ' accepted ' substring after latest session record; exactly one 'Backend warning:' toast per ws connection; rg -c '\\x1b' on lines after latest sing-box session == 0"
        }
      },
      "redTasks": [],
      "codeTasks": [
        "5.1",
        "5.2"
      ],
      "redTests": [],
      "redRun": "",
      "coder": "zpatcher",
      "waiver": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command. 5.1 closes by the floor after shard hygiene merges; 5.2 and the live part of 4.1 are manual checks reported by the operator.",
      "verify": "timeout 10m cargo test --workspace --all-targets -- --test-threads=4 && cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings",
      "notes": [
        "after hygiene merge: add CHANGELOG.md [Unreleased] entries (Added: backend log level + connection log settings, backend warning toasts; Changed: xray/v2ray access log off by default; Fixed: color escapes in backend.log; session record states utc_offset) and update docs/ARCHITECTURE.md settings/logging mentions if present"
      ]
    }
  ],
  "seams": [
    {
      "id": "logging-settings",
      "tasks": [
        "1.1",
        "1.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command. New [logging] section on AppSettings with serde defaults (warning/false); legacy TOML without the section or with a missing key loads defaults; logging joins RuntimeConfigSnapshot, diverges_from and restore_settings so a change while connected shows the pending-restart indication (app.rs:434 already drives it from diverges_from). Reference: RealDelaySettings/HealthCheckSettings settings.rs:113-151,184-187; Language enum settings.rs:105-111; snapshot runtime_snapshot.rs:9-63.",
      "contract": {
        "states": [
          "level-default",
          "level-set",
          "connlog-off",
          "connlog-on",
          "snapshot-equal",
          "snapshot-diverged"
        ],
        "transitions": [
          {
            "input": "TOML without [logging] section (legacy literal as settings.rs:329)",
            "state": "level-default",
            "effect": "forced",
            "evidence": "app-persistence spec: absent section loads backend_level=warning; design Decisions 'Settings shape' #[serde(default)]"
          },
          {
            "input": "TOML without [logging] section",
            "state": "connlog-off",
            "effect": "forced",
            "evidence": "app-persistence spec scenario 'Legacy settings without the section'"
          },
          {
            "input": "[logging] with only connection_log = true (backend_level key absent)",
            "state": "level-default",
            "effect": "forced",
            "evidence": "app-persistence spec: 'When the section or a key is absent' -> container #[serde(default)] on LoggingSettings"
          },
          {
            "input": "[logging] backend_level = \"debug\"",
            "state": "level-set",
            "effect": "set",
            "evidence": "app-persistence spec scenario 'Round-trip'"
          },
          {
            "input": "[logging] connection_log = true",
            "state": "connlog-on",
            "effect": "set",
            "evidence": "app-persistence spec scenario 'Round-trip'"
          },
          {
            "input": "toml::to_string then toml::from_str of settings with Debug/true",
            "state": "level-set",
            "effect": "no-op",
            "evidence": "round-trip preserves values; settings.rs:305-311 pattern"
          },
          {
            "input": "snapshot built from settings, settings unchanged",
            "state": "snapshot-equal",
            "effect": "no-op",
            "evidence": "runtime_snapshot.rs:28-49"
          },
          {
            "input": "settings.logging.backend_level changed after snapshot",
            "state": "snapshot-diverged",
            "effect": "set",
            "evidence": "tasks 1.2; app-persistence scenario 'Change while connected'"
          },
          {
            "input": "settings.logging.connection_log changed after snapshot",
            "state": "snapshot-diverged",
            "effect": "set",
            "evidence": "app-persistence spec: 'Changing either value while connected SHALL count as restart-relevant'"
          },
          {
            "input": "snapshot.restore_settings(&mut settings)",
            "state": "level-set",
            "effect": "forced",
            "evidence": "runtime_snapshot.rs:51-63 restores every runtime field; logging added alongside tun/timeouts"
          },
          {
            "input": "unknown level literal e.g. backend_level = \"trace\"",
            "state": "level-default",
            "effect": "no-op",
            "evidence": "out of spec; serde returns Err like other lowercase enums (Language). No test asserts it; do not add a lenient deserializer"
          }
        ],
        "forbidden": [
          "backend_level serialized as anything other than one of: error, warning, info, debug",
          "RuntimeConfigSnapshot without a logging field while diverges_from ignores settings.logging",
          "LoggingSettings::default() other than { backend_level: Warning, connection_log: false }"
        ],
        "seeding": [
          "level-default/connlog-off: toml::from_str::<AppSettings>(legacy literal copied from settings.rs:329 test) or AppSettings::default()",
          "level-set/connlog-on: AppSettings { logging: LoggingSettings { backend_level: BackendLogLevel::Debug, connection_log: true }, ..AppSettings::default() } (plain data, the persisted shape)",
          "snapshot-equal: make_snapshot(BackendType::Xray, \"/usr/bin/xray\") (runtime_snapshot.rs:87) whose logging is LoggingSettings::default(), vs settings with matching backend (pattern runtime_snapshot.rs:405-420)",
          "snapshot-diverged: mutate settings.logging.backend_level / settings.logging.connection_log after the snapshot, never mutate the snapshot"
        ],
        "budgets": [
          "no new dependency",
          "every existing RuntimeConfigSnapshot literal updated: runtime_snapshot.rs:88,159,235,255,339 and app.rs:616,2231"
        ],
        "names": {
          "enum": "pub enum BackendLogLevel { Error, #[default] Warning, Info, Debug } with #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = \"lowercase\")]",
          "enum_methods": "impl BackendLogLevel { pub const ALL: [BackendLogLevel; 4] = [Error, Warning, Info, Debug]; pub const fn as_str(self) -> &'static str /* error|warning|info|debug */; pub const fn singbox_level(self) -> &'static str /* error|warn|info|debug */ }",
          "struct": "pub struct LoggingSettings { pub backend_level: BackendLogLevel, pub connection_log: bool } with #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(default)]",
          "field": "AppSettings: #[serde(default)] pub logging: LoggingSettings, placed after health_check; AppSettings::default() sets logging: LoggingSettings::default()",
          "reexport": "crates/core/src/models/mod.rs:28 pub use settings::{..., BackendLogLevel, LoggingSettings, ...}",
          "snapshot_field": "RuntimeConfigSnapshot { pub logging: LoggingSettings }; diverges_from adds `|| self.logging != settings.logging`; restore_settings adds `settings.logging = self.logging;`; app.rs:616 adds `logging: self.settings.logging`",
          "tests_1.1": "settings.rs tests: test_default_logging_settings, test_legacy_settings_toml_missing_logging_defaults, test_logging_section_missing_key_defaults, test_logging_settings_toml_roundtrip_debug_connection_log",
          "tests_1.2": "runtime_snapshot.rs tests: test_runtime_config_snapshot_detects_logging_divergence, test_runtime_config_snapshot_restores_logging"
        }
      },
      "codeTasks": [
        "1.1",
        "1.2"
      ]
    },
    {
      "id": "generator-log",
      "tasks": [
        "2.1",
        "2.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command. v2ray/xray log object comes from settings.logging (loglevel = as_str, access = \"none\" unless connection_log, then key omitted); sing-box log object is exactly {\"level\": singbox_level} and ignores connection_log; probe configs (probe.rs:73,101,165) take no settings and stay fixed. Sites: v2ray.rs:86 (shared by xray via generate_v2ray_family_config, xray.rs:33), singbox.rs:41. No other code writes config[\"log\"] (verified). 2.2 adds a debug+connection-log case to both live-binary checks; both binaries are installed on this host (/usr/bin/xray, /usr/bin/sing-box) so the skip branch will not trigger here.",
      "contract": {
        "states": [
          "v2ray-log",
          "xray-log",
          "singbox-log",
          "probe-log"
        ],
        "transitions": [
          {
            "input": "XrayGenerator.generate with AppSettings::default()",
            "state": "xray-log",
            "effect": "set",
            "evidence": "config-generator scenario 'Defaults silence xray access lines': log == {\"loglevel\":\"warning\",\"access\":\"none\"}"
          },
          {
            "input": "V2rayGenerator.generate with level Info, connection_log true",
            "state": "v2ray-log",
            "effect": "set",
            "evidence": "config-generator scenario 'Connection log on for v2ray': loglevel info, access key absent (not empty string, not null)"
          },
          {
            "input": "V2ray/Xray generate with connection_log false, any level",
            "state": "v2ray-log",
            "effect": "set",
            "evidence": "config-generator requirement: access SHALL be \"none\" unless connection log on"
          },
          {
            "input": "SingboxGenerator.generate with level Warning, connection_log true",
            "state": "singbox-log",
            "effect": "set",
            "evidence": "config-generator scenario 'sing-box level mapping': log == {\"level\":\"warn\"} exactly"
          },
          {
            "input": "SingboxGenerator.generate with each BackendLogLevel, connection_log false and true",
            "state": "singbox-log",
            "effect": "set",
            "evidence": "mapping error->error, warning->warn, info->info, debug->debug; connection_log no effect"
          },
          {
            "input": "probe_generator_for(backend).generate(...) for all three backends",
            "state": "probe-log",
            "effect": "no-op",
            "evidence": "probe.rs:73 {\"level\":\"warn\"}, :101 and :165 {\"loglevel\":\"warning\"}; ProbeConfigGenerator::generate takes no AppSettings (probe.rs:26-32)"
          },
          {
            "input": "xray run -test / sing-box check on generated config with level Debug + connection_log true",
            "state": "xray-log",
            "effect": "no-op",
            "evidence": "tasks 2.2; xray_check.rs:98-170, singbox_check.rs:193+"
          }
        ],
        "forbidden": [
          "v2ray/xray log containing \"access\" when connection_log is true",
          "v2ray/xray log without \"access\":\"none\" when connection_log is false",
          "sing-box log containing any key other than \"level\" (no timestamp/output/disabled)",
          "sing-box level literal \"warning\"",
          "probe generators reading AppSettings.logging"
        ],
        "seeding": [
          "settings via AppSettings { logging: LoggingSettings { .. }, ..default_settings() } in each generator's test module (v2ray.rs default_settings()/vless_node() at :1010, xray.rs tests, singbox.rs tests)",
          "live checks: new case pushed into existing cases vecs, not a new test fn"
        ],
        "budgets": [
          "log object exactly 2 keys (xray/v2ray off), 1 key (xray/v2ray on), 1 key (sing-box)",
          "integration check run under timeout 5m"
        ],
        "names": {
          "helper": "fn log_object(logging: &LoggingSettings) -> Value in crates/core/src/config/v2ray.rs (private); sing-box inline json!({ \"level\": settings.logging.backend_level.singbox_level() })",
          "tests_2.1": "xray.rs: xray_log_defaults_silence_access; v2ray.rs: v2ray_log_info_with_connection_log_omits_access, v2ray_log_off_sets_access_none_at_every_level; singbox.rs: singbox_log_maps_levels_and_ignores_connection_log; probe.rs: probe_configs_keep_fixed_log_objects",
          "tests_2.2": "xray_check.rs generated_xray_configs_pass_xray_test gains case \"log-debug-connection-log\"; singbox_check.rs generated_singbox_configs_pass_sing_box_check gains case \"log-debug\" (level Debug, connection_log true)"
        }
      },
      "codeTasks": [
        "2.1",
        "2.2"
      ]
    },
    {
      "id": "log-hygiene",
      "tasks": [
        "3.1",
        "3.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command. One escape stripper in core, applied once per line in capture_output (stdout and stderr readers, manager.rs:872-898) and log_helper (manager.rs:1019-1027) before write_stream_line, LogLine construction and broadcast; the session record gains utc_offset=±HH:MM. The stop-failover change already landed two private strippers (ui connection.rs:1026-1049 CSI-only, process manager.rs:1110-1129 without_ansi); per design Non-Goals ('reuse its helper instead of adding a second one') the core function is that helper lifted and extended with OSC, and both private copies are deleted in favor of v2ray_rs_core::ansi::strip_ansi (CandidateFailure::new uses .into_owned(); last_error_line keeps working because the buffer is now already stripped; test last_error_matches_through_ansi manager.rs:1598 stays green). process crate has no chrono dependency, so the offset formatter lives in core rotating_log.rs next to the writer's UTC prefix (no Cargo.toml change).",
      "contract": {
        "states": [
          "borrowed",
          "stripped",
          "file-clean",
          "buffer-clean",
          "stream-clean",
          "offset-recorded"
        ],
        "transitions": [
          {
            "input": "line with no U+001B",
            "state": "borrowed",
            "effect": "no-op",
            "evidence": "tasks 3.1 'plain line unchanged and borrowed'; return Cow::Borrowed"
          },
          {
            "input": "CSI: ESC '[' then 0x30-0x3F* then 0x20-0x2F* then one final 0x40-0x7E",
            "state": "stripped",
            "effect": "clear",
            "evidence": "design 'Strip escapes at capture'; existing connection.rs:1035-1046 grammar. Whole sequence removed"
          },
          {
            "input": "CSI whose next char after params/intermediates is not a final byte (e.g. ESC [ 3 \\u{80})",
            "state": "stripped",
            "effect": "clear",
            "evidence": "connection.rs:1046 next_if: consumed prefix removed, the offending char kept"
          },
          {
            "input": "truncated CSI at end of line (\"WARN\\u{1b}[3\" or \"x\\u{1b}[\")",
            "state": "stripped",
            "effect": "clear",
            "evidence": "tasks 3.1 'truncated escape at end of line' -> \"WARN\" / \"x\""
          },
          {
            "input": "OSC: ESC ']' ... terminated by BEL 0x07 or ST (ESC backslash)",
            "state": "stripped",
            "effect": "clear",
            "evidence": "design: OSC ESC ] ... BEL|ESC backslash; terminator removed with the body"
          },
          {
            "input": "unterminated OSC to end of line",
            "state": "stripped",
            "effect": "clear",
            "evidence": "same rule as truncated CSI: removed through end of line"
          },
          {
            "input": "lone ESC followed by any other char, or ESC as last char",
            "state": "stripped",
            "effect": "clear",
            "evidence": "connection.rs:1035-1037: ESC dropped, following char kept"
          },
          {
            "input": "sing-box stderr \"\\u{1b}[31mFATAL\\u{1b}[0m[0000] start service: boom\" via stub backend",
            "state": "file-clean",
            "effect": "forced",
            "evidence": "diagnostic-logs scenario 'sing-box color codes'; backend.log line contains \"stderr FATAL[0000] start service: boom\" and no 0x1b byte anywhere in the file"
          },
          {
            "input": "same stub line",
            "state": "buffer-clean",
            "effect": "forced",
            "evidence": "design: before file, buffer, and stream; LogBuffer content == \"FATAL[0000] start service: boom\""
          },
          {
            "input": "same stub line",
            "state": "stream-clean",
            "effect": "forced",
            "evidence": "ProcessEvent::LogLine from subscribe_logs() carries stripped content"
          },
          {
            "input": "HelperRun { output: [\"\\u{1b}[1mlink up\\u{1b}[0m\"], result: Err(\"\\u{1b}[31mno\\u{1b}[0m\") }",
            "state": "file-clean",
            "effect": "forced",
            "evidence": "manager.rs:1013-1028; both output lines and the '{verb} failed: {e}' line are stripped before write/buffer/emit"
          },
          {
            "input": "launch() with log writer attached",
            "state": "offset-recorded",
            "effect": "set",
            "evidence": "manager.rs:681-702; record = \"backend={backend} version={version} node={node} tun={tun} utc_offset={offset}\" then optional ' ' + session_fields"
          },
          {
            "input": "format_utc_offset(10800) / (0) / (-19800) / (20700)",
            "state": "offset-recorded",
            "effect": "set",
            "evidence": "\"+03:00\" / \"+00:00\" / \"-05:30\" / \"+05:45\"; seconds component dropped; diagnostic-logs scenario 'UTC offset recorded'"
          },
          {
            "input": "backend timestamp text inside the line (xray '2026/09/14 09:57:30.532567 ')",
            "state": "file-clean",
            "effect": "no-op",
            "evidence": "design 'UTC offset, not rewritten clocks': backend text byte-identical apart from escapes"
          }
        ],
        "forbidden": [
          "any U+001B in the return value of strip_ansi",
          "Cow::Owned returned for an input without U+001B",
          "an escape byte in backend.log, LogBuffer or broadcast content originating from capture_output or log_helper",
          "stripping applied separately per sink (must be one call per line, result reused for file, buffer, stream)",
          "rewriting or removing the backend's own timestamp, or changing RotatingFileWriter's RFC 3339 prefix",
          "utc_offset read from Utc or computed per stdout line; it is one field per session record",
          "keeping connection.rs strip_ansi or manager.rs without_ansi alongside the core one"
        ],
        "seeding": [
          "borrowed/stripped: direct calls to v2ray_rs_core::ansi::strip_ansi with literals",
          "file-clean/buffer-clean/stream-clean: manager_for(&dir, script) (manager.rs:1185) .with_log_file(Some(backend_log(dir.path()))), subscribe_logs() before start, set_auto_restart(false), start_with_connection(None), wait_and_handle_exit(); script emits via printf '\\033[31mFATAL\\033[0m[0000] start service: boom\\n' >&2 and printf '\\033]0;t\\007plain out\\n'; never push raw lines into LogBuffer directly",
          "helper path: call mgr.log_helper(\"xray-up\", &HelperRun { output, result, timed_out: false }) from the manager.rs test module on a manager with a log file",
          "offset-recorded: same stub start; assert session line contains format!(\"utc_offset={}\", local_utc_offset()) and matches /utc_offset=[+-]\\d{2}:\\d{2}/; the +03:00 scenario is covered by format_utc_offset(10800) (TZ is process-global, never set TZ in tests)"
        ],
        "budgets": [
          "strip_ansi single pass O(len), zero allocation when no ESC",
          "one strip_ansi call per captured line",
          "process tests under timeout 5m, stub scripts exit within 1s"
        ],
        "names": {
          "module": "crates/core/src/ansi.rs; `pub mod ansi;` in crates/core/src/lib.rs",
          "fn": "pub fn strip_ansi(line: &str) -> Cow<'_, str>",
          "offset_fns": "crates/core/src/rotating_log.rs: pub fn format_utc_offset(seconds_east: i32) -> String; pub fn local_utc_offset() -> String { format_utc_offset(chrono::Local::now().offset().local_minus_utc()) }",
          "session_field": "utc_offset=",
          "removed": "crates/ui/src/connection.rs fn strip_ansi (1026-1049), crates/process/src/manager.rs fn without_ansi (1110-1129)",
          "tests_3.1": "ansi.rs: strip_ansi_removes_singbox_color_codes, strip_ansi_plain_line_is_borrowed, strip_ansi_truncated_csi_at_end_drops_tail, strip_ansi_removes_osc_terminated_by_bel_and_st, strip_ansi_unterminated_osc_drops_tail, strip_ansi_drops_lone_escape; rotating_log.rs: format_utc_offset_signs_hours_minutes",
          "tests_3.2": "manager.rs: backend_escapes_stripped_before_file_buffer_and_stream, helper_lines_stripped_of_escapes, session_record_states_utc_offset"
        }
      },
      "codeTasks": [
        "3.1",
        "3.2"
      ]
    },
    {
      "id": "ui-diagnostics",
      "tasks": [
        "4.1"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command. System page gains a Diagnostics group: ComboRow 'Backend log level' (labels = BackendLogLevel::ALL as_str, lowercase) and SwitchRow 'Connection log'. The backend is chosen on the Network page inside the same dialog, so sensitivity is live: build_system_page takes &SettingsObservers and subscribes like dns.rs:499 via subscribe_settings (mod.rs:108); initial state from state.borrow() at build time. Switch insensitive with the sing-box note while settings.backend.backend_type == SingBox; sensitive with the default note otherwise. Level combo always sensitive. Writes go through emit(&st,&cb) like system.rs:54-80. 'Verified live for xray and sing-box' is MANUAL (tracked under verification).",
      "contract": {
        "states": [
          "level-shown",
          "switch-sensitive",
          "switch-insensitive"
        ],
        "transitions": [
          {
            "input": "page built with xray (AppSettings::default())",
            "state": "level-shown",
            "effect": "set",
            "evidence": "diagnostic-logs scenario 'Controls shown': combo selected index 1 ('warning'), switch inactive"
          },
          {
            "input": "page built with xray or v2ray",
            "state": "switch-sensitive",
            "effect": "set",
            "evidence": "scenario 'Controls shown': visible and sensitive; subtitle CONNECTION_LOG_NOTE"
          },
          {
            "input": "page built with sing-box",
            "state": "switch-insensitive",
            "effect": "set",
            "evidence": "scenario 'sing-box note'; subtitle SINGBOX_CONNECTION_LOG_NOTE"
          },
          {
            "input": "settings observer fired with backend changed to sing-box (Network page selection)",
            "state": "switch-insensitive",
            "effect": "set",
            "evidence": "mod.rs:48-59 fan-out; dns.rs:499-501 live-backend pattern"
          },
          {
            "input": "settings observer fired with backend changed back to xray/v2ray",
            "state": "switch-sensitive",
            "effect": "set",
            "evidence": "same fan-out"
          },
          {
            "input": "combo selection changed",
            "state": "level-shown",
            "effect": "set",
            "evidence": "writes state.logging.backend_level = BackendLogLevel::ALL[selected], then emit"
          },
          {
            "input": "switch toggled (only reachable while sensitive)",
            "state": "switch-sensitive",
            "effect": "no-op",
            "evidence": "writes state.logging.connection_log, then emit; sensitivity unchanged"
          },
          {
            "input": "sing-box selected while connection_log persisted true",
            "state": "switch-insensitive",
            "effect": "no-op",
            "evidence": "switch keeps active=true, insensitive; generator ignores it for sing-box; value not rewritten"
          }
        ],
        "forbidden": [
          "switch sensitive while settings.backend.backend_type == SingBox",
          "writing connection_log=false when the backend becomes sing-box",
          "observer callback holding state.borrow() across emit (mod.rs:97-99 invariant)"
        ],
        "seeding": [
          "pure: connection_log_note(BackendType::SingBox / Xray / V2ray)",
          "GTK: crate::gtk_test::run (gtk_test.rs:13) building the group via build_diagnostics_group with Rc<RefCell<AppSettings>>, a no-op SettingsCallback and a SettingsObservers vec; backend change seeded by invoking the observers with a settings clone whose backend_type is SingBox (the same path emit uses), never by calling set_sensitive in the test; skips with 'no display, skipping' when headless"
        ],
        "budgets": [
          "no new crate deps",
          "one observer registration"
        ],
        "names": {
          "signature": "pub(super) fn build_system_page(state: &Rc<RefCell<AppSettings>>, cb: &SettingsCallback, observers: &SettingsObservers) -> adw::PreferencesPage; mod.rs:63 passes &settings_observers",
          "group_builder": "fn build_diagnostics_group(state: &Rc<RefCell<AppSettings>>, cb: &SettingsCallback, observers: &SettingsObservers) -> (adw::PreferencesGroup, adw::ComboRow, adw::SwitchRow)",
          "group_title": "Diagnostics",
          "combo_title": "Backend log level",
          "switch_title": "Connection log",
          "note_fn": "fn connection_log_note(backend: BackendType) -> (bool, &'static str) /* (sensitive, subtitle) */",
          "notes": "const CONNECTION_LOG_NOTE: &str = \"Write accepted connections to the backend log\"; const SINGBOX_CONNECTION_LOG_NOTE: &str = \"sing-box writes connection lines at the info and debug levels\"",
          "tests_4.1": "system.rs: connection_log_note_insensitive_for_singbox_only, diagnostics_group_tracks_backend_selection (gtk_test::run)"
        }
      },
      "codeTasks": [
        "4.1"
      ]
    },
    {
      "id": "backend-warnings",
      "tasks": [
        "4.2",
        "4.3"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command. Pure matcher + per-connection once-set in new module crates/ui/src/backend_warning.rs (reference shape: health.rs:63-80 DnsFailureWindow observed from the same forwarder). The set is created once per spawn_with task before the candidate loop (next to capture_notice_sent, connection.rs:211) as Arc<std::sync::Mutex<BackendWarnings>>, cloned into every candidate's log_forwarder (connection.rs:435-464); it survives failover to the next candidate and in-place crash respawns, and is dropped when the task ends, so a new Connect (new generation) starts empty. Forwarder order per LogLine: DNS observe, emit ProcessLogLine (unchanged), then if toast_for returns Some emit AppMsg::ShowToast. Lines arrive already escape-stripped (log-hygiene seam). 4.3's stub-backend test uses the existing connection.rs harness: stub() :1215, connect() :1248, drain_within() :2296 (collects toasts) with an xray stub script and a ready listener on socks_port.",
      "contract": {
        "states": [
          "no-match",
          "matched-deprecated",
          "matched-reality",
          "toasted",
          "suppressed"
        ],
        "transitions": [
          {
            "input": "(Xray, \"2026/09/15 08:51:54.138024 [Warning] common/errors: The feature WebSocket transport (with ALPN http/1.1, etc.) is deprecated, not recommended for using and might be removed. Please migrate to XHTTP H2 & H3 as soon as possible.\")",
            "state": "matched-deprecated",
            "effect": "set",
            "evidence": "diagnostic-logs scenario 'xray WebSocket deprecation'; recorded format in backend.log 2026-09-15"
          },
          {
            "input": "(V2ray, any line containing \"is deprecated\")",
            "state": "matched-deprecated",
            "effect": "set",
            "evidence": "design Warning patterns: xray/v2ray `is deprecated`"
          },
          {
            "input": "(Xray, \"2026/09/14 09:57:30.532567 [Error] [1944052120] transport/internet/reality: REALITY: received real certificate (potential MITM or redirection)\")",
            "state": "matched-reality",
            "effect": "set",
            "evidence": "spec REALITY pattern; recorded backend.log 2026-09-14"
          },
          {
            "input": "(SingBox, \"WARN[0000] implicit default HTTP client using default outbound for remote rule-sets is deprecated in sing-box 1.14.0 and will be removed in sing-box 1.16.0.\")",
            "state": "matched-deprecated",
            "effect": "set",
            "evidence": "spec sing-box WARN deprecated; recorded stripped form (sing-box writes no timestamp when piped)"
          },
          {
            "input": "(Xray, \"2026/09/14 09:57:31.000000 [Warning] [123] app/dispatcher: failed to dial tcp 203.0.113.1:443 > dial tcp: i/o timeout\")",
            "state": "no-match",
            "effect": "no-op",
            "evidence": "tasks 4.2 ordinary [Warning] dial failure; design rejects toasting every [Warning]"
          },
          {
            "input": "(Xray, \"[Warning] core: Xray 26.9.9 started\")",
            "state": "no-match",
            "effect": "no-op",
            "evidence": "recorded startup line"
          },
          {
            "input": "(SingBox, \"INFO[0000] ... deprecated ...\") or (SingBox, WebSocket xray line)",
            "state": "no-match",
            "effect": "no-op",
            "evidence": "sing-box needs both WARN and deprecated; xray-only needles not applied to sing-box"
          },
          {
            "input": "(V2ray, REALITY MITM line)",
            "state": "no-match",
            "effect": "no-op",
            "evidence": "design: MITM needle is xray only (v2ray refuses REALITY nodes, bcf47b2/05f4e90)"
          },
          {
            "input": "BackendWarnings::toast_for on first line matching a PatternId",
            "state": "toasted",
            "effect": "set",
            "evidence": "spec: first matching line toasts, quoting it"
          },
          {
            "input": "toast_for on a later line with an already-seen PatternId (same connection)",
            "state": "suppressed",
            "effect": "no-op",
            "evidence": "scenario 'Repeated REALITY warning': exactly one toast"
          },
          {
            "input": "toast_for on a line of a different PatternId after one toasted",
            "state": "toasted",
            "effect": "set",
            "evidence": "at most once per pattern, not per connection"
          },
          {
            "input": "failover to next candidate or in-place respawn within the same Connect",
            "state": "suppressed",
            "effect": "no-op",
            "evidence": "shared Arc set outside the candidate loop, like capture_notice_sent connection.rs:209-211"
          },
          {
            "input": "new Connect (new spawn_with task / generation)",
            "state": "no-match",
            "effect": "clear",
            "evidence": "set owned by the task; design 'once per connection'"
          },
          {
            "input": "stub xray connection echoing the REALITY line three times",
            "state": "toasted",
            "effect": "forced",
            "evidence": "tasks 4.3: toasts == [expected] exactly once; all three lines still arrive as ProcessLogLine (scenario 'Warnings are never access noise')"
          }
        ],
        "forbidden": [
          "more than one ShowToast per PatternId per connection",
          "suppressing or reordering the ProcessLogLine for a matched line",
          "toast quote longer than 160 chars",
          "per-candidate or per-forwarder set (would re-toast on failover)",
          "matching against raw lines with escapes stripped a second time in the ui (process layer owns stripping)"
        ],
        "seeding": [
          "matcher: direct match_backend_warning(BackendType, literal) calls with the recorded lines above",
          "once-set: BackendWarnings::new(BackendType::Xray) then toast_for x3 -> Some, None, None",
          "4.3: stub(r#\"[ \"$1\" = version ] && echo \"Xray 26.6.27\" && exit 0; [ \"$2\" = -test ] && exit 0; for i in 1 2 3; do echo \"2026/09/14 09:57:30.532567 [Error] [1944052120] transport/internet/reality: REALITY: received real certificate (potential MITM or redirection)\"; done; exec sleep 30\"#); settings = AppSettings default with backend Xray and socks_port bound to a live TcpListener (xray analog of ready_singbox_settings :1320, tun off); connect(&stub, settings, vec![candidate(\"203.0.113.1\")]); wait until three ProcessLogLine containing \"potential MITM\" were received, handle.stop(StopReason::UserStop), then drain_within collects the rest"
        ],
        "budgets": [
          "quote <= 160 chars (chars, not bytes)",
          "<= 1 toast per PatternId per connection",
          "4.3 test bounded by RECV_TIMEOUT 20s per message and timeout 5m overall"
        ],
        "names": {
          "module": "crates/ui/src/backend_warning.rs; `mod backend_warning;` in crates/ui/src/lib.rs",
          "pattern_id": "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] pub(crate) enum PatternId { Deprecated, RealityMitm }",
          "table": "const PATTERNS: &[(BackendType, PatternId, &[&str])] = &[(BackendType::Xray, PatternId::Deprecated, &[\"is deprecated\"]), (BackendType::V2ray, PatternId::Deprecated, &[\"is deprecated\"]), (BackendType::Xray, PatternId::RealityMitm, &[\"potential MITM or redirection\"]), (BackendType::SingBox, PatternId::Deprecated, &[\"WARN\", \"deprecated\"])]; first row whose backend equals and whose needles are all contained (case-sensitive) wins",
          "matcher": "pub(crate) fn match_backend_warning(backend: BackendType, line: &str) -> Option<PatternId>",
          "toast_fn": "pub(crate) fn warning_toast(line: &str) -> String = format!(\"Backend warning: {quote}\") where quote = crate::connection::strip_leading_timestamp(line.trim()) (made pub(crate); removes one leading YYYY/MM/DD HH:MM:SS[.frac] plus following spaces, the xray/v2ray form; sing-box lines carry no timestamp and are quoted as-is incl. WARN[0000]); if quote.chars().count() > 160 then first 159 chars + '…' (exactly 160 chars)",
          "set": "pub(crate) struct BackendWarnings { backend: BackendType, seen: HashSet<PatternId> }; pub(crate) fn new(backend: BackendType) -> Self; pub(crate) fn toast_for(&mut self, line: &str) -> Option<String>",
          "forwarder_binding": "let warnings = Arc::new(std::sync::Mutex::new(BackendWarnings::new(settings.backend.backend_type))); before 'candidates loop; Arc::clone into log_forwarder",
          "expected_reality_toast": "Backend warning: [Error] [1944052120] transport/internet/reality: REALITY: received real certificate (potential MITM or redirection)",
          "tests_4.2": "backend_warning.rs: matches_xray_websocket_deprecation, matches_xray_reality_mitm, matches_singbox_warn_deprecation, ignores_ordinary_xray_dial_warning, ignores_patterns_of_other_backends, toast_drops_xray_timestamp, toast_truncates_to_160_chars, toasts_once_per_pattern",
          "tests_4.3": "connection.rs: reality_warning_toasts_once_per_connection"
        }
      },
      "codeTasks": [
        "4.2",
        "4.3"
      ]
    },
    {
      "id": "verification",
      "tasks": [
        "5.1",
        "5.2"
      ],
      "summary": "NO-RED-WAIVER: Rust stack, no test-writer; tests are the first codeTasks. NO-TESTER-WAIVER: Rust stack, closes by verify command. 5.1 is the workspace floor. 5.2 is MANUAL and cannot be closed by a command: live xray TUN with defaults for 10 minutes of browsing (no 'accepted' lines in ~/.local/share/v2ray-rs/state/logs/backend.log after the session line; one deprecation toast for a ws node), connection log on + restart brings access lines back, sing-box session writes no escape bytes in new lines. The 'verified live for xray and sing-box' clause of 4.1 (System page controls, sing-box insensitive switch with note) is also MANUAL and belongs to this pass.",
      "contract": {
        "states": [
          "floor-green",
          "live-verified"
        ],
        "transitions": [
          {
            "input": "fullFloor command",
            "state": "floor-green",
            "effect": "set",
            "evidence": "tasks 5.1"
          },
          {
            "input": "manual live session per 5.2 and 4.1 live check",
            "state": "live-verified",
            "effect": "set",
            "evidence": "tasks 5.2, 4.1; manual"
          }
        ],
        "forbidden": [
          "marking 5.2 or the live part of 4.1 done from automated runs alone"
        ],
        "seeding": [
          "live: installed /usr/bin/xray and /usr/bin/sing-box with a real ws node; counting new lines only after the latest session record"
        ],
        "budgets": [
          "5.2 live window 10 minutes",
          "floor under timeout 10m"
        ],
        "names": {
          "live_checks": "no ' accepted ' substring after latest session record; exactly one 'Backend warning:' toast per ws connection; rg -c '\\x1b' on lines after latest sing-box session == 0"
        }
      },
      "codeTasks": [
        "5.1",
        "5.2"
      ]
    }
  ],
  "requirements": [
    {
      "shall": "The System page of Preferences SHALL offer a backend log level selector",
      "tests": [
        "system.rs diagnostics_group_tracks_backend_selection",
        "system.rs connection_log_note_insensitive_for_singbox_only"
      ]
    },
    {
      "shall": "an off connection log switch SHALL be visible and sensitive",
      "tests": [
        "system.rs diagnostics_group_tracks_backend_selection"
      ]
    },
    {
      "shall": "the connection log switch SHALL be insensitive and its note SHALL say connection lines appear",
      "tests": [
        "system.rs connection_log_note_insensitive_for_singbox_only",
        "system.rs diagnostics_group_tracks_backend_selection"
      ]
    },
    {
      "shall": "The system SHALL remove ANSI escape sequences from every backend and route-helper line",
      "tests": [
        "ansi.rs strip_ansi_*",
        "manager.rs backend_escapes_stripped_before_file_buffer_and_stream",
        "manager.rs helper_lines_stripped_of_escapes"
      ]
    },
    {
      "shall": "`backend.log` SHALL contain `FATAL[0000] start service: …` with no escape bytes",
      "tests": [
        "manager.rs backend_escapes_stripped_before_file_buffer_and_stream",
        "ansi.rs strip_ansi_removes_singbox_color_codes"
      ]
    },
    {
      "shall": "its session record SHALL contain `utc_offset=+03:00`",
      "tests": [
        "rotating_log.rs format_utc_offset_signs_hours_minutes",
        "manager.rs session_record_states_utc_offset"
      ]
    },
    {
      "shall": "The system SHALL show a non-blocking toast for the first backend line in a connection",
      "tests": [
        "backend_warning.rs matches_*",
        "backend_warning.rs toast_*",
        "connection.rs reality_warning_toasts_once_per_connection"
      ]
    },
    {
      "shall": "one toast SHALL quote that the WebSocket transport is deprecated",
      "tests": [
        "backend_warning.rs matches_xray_websocket_deprecation",
        "backend_warning.rs toast_drops_xray_timestamp"
      ]
    },
    {
      "shall": "exactly one toast SHALL be shown for it",
      "tests": [
        "backend_warning.rs toasts_once_per_pattern",
        "connection.rs reality_warning_toasts_once_per_connection"
      ]
    },
    {
      "shall": "warning and error lines SHALL still reach `backend.log` and the logs page",
      "tests": [
        "xray.rs xray_log_defaults_silence_access",
        "connection.rs reality_warning_toasts_once_per_connection"
      ]
    },
    {
      "shall": "The generated connection config SHALL set the backend's log verbosity",
      "tests": [
        "v2ray.rs v2ray_log_off_sets_access_none_at_every_level",
        "v2ray.rs v2ray_log_info_with_connection_log_omits_access",
        "singbox.rs singbox_log_maps_levels_and_ignores_connection_log",
        "probe.rs probe_configs_keep_fixed_log_objects",
        "xray_check.rs generated_xray_configs_pass_xray_test",
        "singbox_check.rs generated_singbox_configs_pass_sing_box_check"
      ]
    },
    {
      "shall": "the `log` object SHALL be `{\"loglevel\": \"warning\", \"access\": \"none\"}`",
      "tests": [
        "xray.rs xray_log_defaults_silence_access"
      ]
    },
    {
      "shall": "`log.loglevel` SHALL be `\"info\"` and `log.access` SHALL be absent",
      "tests": [
        "v2ray.rs v2ray_log_info_with_connection_log_omits_access"
      ]
    },
    {
      "shall": "the `log` object SHALL be `{\"level\": \"warn\"}`",
      "tests": [
        "singbox.rs singbox_log_maps_levels_and_ignores_connection_log"
      ]
    },
    {
      "shall": "The system SHALL persist backend logging preferences in `settings.toml`",
      "tests": [
        "settings.rs test_logging_settings_toml_roundtrip_debug_connection_log",
        "settings.rs test_legacy_settings_toml_missing_logging_defaults",
        "settings.rs test_logging_section_missing_key_defaults",
        "runtime_snapshot.rs test_runtime_config_snapshot_detects_logging_divergence"
      ]
    },
    {
      "shall": "settings SHALL load with `backend_level = \"warning\"` and `connection_log = false`",
      "tests": [
        "settings.rs test_legacy_settings_toml_missing_logging_defaults"
      ]
    },
    {
      "shall": "the reloaded settings SHALL contain `backend_level = \"debug\"`",
      "tests": [
        "settings.rs test_logging_settings_toml_roundtrip_debug_connection_log"
      ]
    },
    {
      "shall": "the pending-restart indication SHALL appear as for other config-changing settings",
      "tests": [
        "runtime_snapshot.rs test_runtime_config_snapshot_detects_logging_divergence",
        "runtime_snapshot.rs test_runtime_config_snapshot_restores_logging"
      ]
    }
  ],
  "testHarness": [
    "fixtures::default_settings / vless_node / ss_node — crates/core/src/config/test_fixtures.rs:5 — AppSettings::default() and sample nodes for generator unit tests",
    "make_snapshot — crates/core/src/runtime_snapshot.rs:87 — RuntimeConfigSnapshot literal for backend/binary",
    "xray check/isolate/check_with_nodes — crates/core/tests/xray_check.rs:29,41,53 — runs installed xray `run -test` on generated config, skips when absent (L100)",
    "singbox check/check_with_rules/check_starts — crates/core/tests/singbox_check.rs:49,53,89 — runs installed sing-box check/start, skips when absent (L134,L195)",
    "write_script / manager_for / backend_log / wait_for_lines — crates/process/src/manager.rs:1177,1185,1195,1669 — fake /bin/sh backend + ProcessManager + backend.log writer",
    "stub / request / connect / ready_singbox_settings / drain_within — crates/ui/src/connection.rs:1215,1256,1248,1320,2296 — fake backend connection task; drain_within collects ShowToast messages",
    "gtk_test::run — crates/ui/src/gtk_test.rs:13 — runs GTK test body on the single shared GTK thread, skips without display"
  ],
  "floor": "timeout 10m cargo test --workspace --all-targets -- --test-threads=4 && cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings",
  "risks": [
    "Design context drift: design.md says 'No escape stripping exists anywhere' but stop-failover landed ui connection.rs:1026-1049 strip_ansi and process manager.rs:1110-1129 without_ansi. Resolved per the design's own Non-Goal (reuse, no second helper): lift into core ansi::strip_ansi and delete both. Existing tests failure_key_ignores_xray_timestamps (connection.rs:1387), the FATAL summary tests (connection.rs:1371-1505) and last_error_matches_through_ansi (manager.rs:1598) must stay green.",
    "process crate has no chrono dependency; offset formatting placed in core rotating_log.rs (format_utc_offset/local_utc_offset) to avoid a Cargo.toml change.",
    "Pending change surface-effective-dns also edits write_session_record surroundings (records after the session line); utc_offset goes inside the session line itself, so textual merge conflict only.",
    "chrono::Local reads TZ at call time; tests never set TZ (process-global) and assert against local_utc_offset() plus the pure formatter.",
    "gtk test diagnostics_group_tracks_backend_selection skips headless ('no display, skipping'); connection_log_note pure test carries the sing-box rule without a display.",
    "Xray access log off by default is a user-visible behavior change; switch note + CHANGELOG [Unreleased] entry recommended.",
    "Plan review round 1: session_record_appends_caller_fields assertion amended in log-hygiene; CHANGELOG/ARCHITECTURE owned by verification chunk."
  ],
  "planReview": {
    "verdict": "pass",
    "reviewer": "zarchitect",
    "rounds": 2
  }
}
```
