## 1. Settings

- [x] 1.1 `LoggingSettings { backend_level, connection_log }` + `BackendLogLevel` in `crates/core/src/models/settings.rs` with serde defaults; tests: legacy TOML without `[logging]` loads defaults, round-trip `debug`/`true`; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [x] 1.2 Add `logging` to `RuntimeConfigSnapshot` and `diverges_from`; test: changing `backend_level` diverges

## 2. Generators

- [x] 2.1 v2ray/xray `log` object from settings (`loglevel`, `access: "none"` unless `connection_log`); sing-box `level` mapping; probe configs untouched; unit tests for the three config-generator scenarios; same core test command green
- [x] 2.2 `crates/core/tests/xray_check.rs` / `singbox_check.rs` still pass against installed binaries with default and `debug` settings (skip notice checked when a binary is absent)

## 3. Log hygiene

- [x] 3.1 `strip_ansi` in `v2ray-rs-core` (CSI and OSC); unit tests: sing-box `\x1b[31mFATAL\x1b[0m[0000] …`, plain line unchanged and borrowed, truncated escape at end of line
- [x] 3.2 Apply in `capture_output` and `log_helper` before file, buffer and stream; session record gains `utc_offset=`; stub-backend test emitting escapes → `backend.log` has none, session line has `utc_offset=`; `timeout 5m cargo test -p v2ray-rs-process -- --test-threads=4` green

## 4. UI

- [x] 4.1 Diagnostics group on the System page: level combo and connection-log switch, switch insensitive with note for sing-box; verified live for xray and sing-box
- [x] 4.2 Pure `match_backend_warning(backend, line) -> Option<PatternId>` with the pattern table; unit tests with the recorded xray WebSocket, REALITY and sing-box deprecation lines and an ordinary `[Warning]` dial failure (no match)
- [x] 4.3 Connection log forwarder toasts once per pattern per connection via `AppMsg::ShowToast`; stub-backend test: three REALITY lines → one toast message

## 5. Verification

- [x] 5.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [x] 5.2 Live xray TUN with defaults for 10 minutes of browsing: no `accepted` lines in `backend.log`, deprecation toast once for a ws node; connection log on + restart → access lines return; sing-box session writes no escape bytes (`rg -c '\x1b' backend.log` → 0 for new lines)
