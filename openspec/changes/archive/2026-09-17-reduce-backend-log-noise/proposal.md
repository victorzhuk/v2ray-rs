## Why

`backend.log` is mostly noise. Across its current rotation set (2026-09-13 08:59 to 2026-09-15 10:11 UTC, about 49 hours in 82,190 lines), 68,463 lines (83%) are xray access lines (`from tcp:… accepted …`), because the generated xray/v2ray config sets only `loglevel: warning` and leaves the access log on. Three 5 MiB rotations therefore hold about two days, and the logs page fills with the same lines. sing-box writes color escapes into the file (58 lines such as `\x1b[31mFATAL\x1b[0m`). Every file line carries the writer's UTC timestamp while xray prefixes its own local time (`2026/09/14 09:57:30` next to `06:57:30Z`), with nothing recording the offset. Backend warnings that need action — `The feature WebSocket transport … is deprecated … Please migrate to XHTTP`, `REALITY: received real certificate (potential MITM or redirection)` — are buried among access lines and never reach the user.

## What Changes

- New settings: backend log level (`error`, `warning`, `info`, `debug`; default `warning`) and a connection log switch (default off), on the System page of Preferences.
- xray and v2ray configs emit `log.loglevel` from the level and `log.access: "none"` unless the connection log is on. **Behavior change:** access lines no longer appear by default.
- sing-box configs emit `log.level` from the level (`warning` → `warn`). sing-box has no separate access log; its connection lines appear at `info` and `debug`, so the connection log switch is insensitive for sing-box with a note saying so.
- Color escape sequences are removed from backend and helper lines before they are written to `backend.log` or shown on the logs page.
- Backend timestamps are left as the backend wrote them; each session record states the local UTC offset once.
- The first backend line per connection matching a known deprecation or security warning is shown as a non-blocking toast.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `config-generator`: new requirement for backend log settings in generated configs.
- `app-persistence`: new requirement for the logging settings section.
- `diagnostic-logs`: new requirements for escape-free lines, the UTC offset, backend log controls, and surfaced backend warnings.

## Impact

- `crates/core/src/models/settings.rs` — `logging` section with serde defaults.
- `crates/core/src/config/v2ray.rs`, `singbox.rs` — `log` object from settings; probe configs unchanged.
- `crates/core/src/runtime_snapshot.rs` — logging settings join the restart-relevant snapshot.
- `crates/process/src/manager.rs` — escape stripping at capture, UTC offset in the session record.
- `crates/ui/src/preferences/system.rs` — Diagnostics group.
- `crates/ui/src/connection.rs` — warning-pattern match on the log stream, toast once per pattern per connection.
