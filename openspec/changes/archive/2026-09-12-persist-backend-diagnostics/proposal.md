# Persist application and backend logs
OVERSIZE: one cohesive capability — both log files share the in-house rotating writer (1.1), and the toast streaks / session records only mean something relative to the installed logger; splitting strands the writer from its consumers.

## Why

Diagnosing the xray restart loop on 2026-09-11 required reconstructing crashes from NetworkManager's journal, because the app keeps no record of its own:

- Backend stdout/stderr lives only in a 10,000-line in-memory buffer; it is gone when the app exits, and a crash's reason is never written anywhere durable.
- No logger is installed, so every `log::warn!`/`log::error!` in the workspace — geodata refresh failures, subscription auto-update failures, failed settings saves, route-recovery errors — is discarded.
- The app is launched with stdout and stderr on `/dev/null`, so the route helper's messages, which inherit them, vanish too.
- Background failures show no toast: a geodata refresh that failed is first noticed as a sing-box fatal at connect time, a subscription that stopped updating is never noticed at all.

## What Changes

- The application installs a logger at startup that writes records to a size-rotated file under the profile's state directory and to stderr.
- Backend output is appended to its own size-rotated file, bracketed by a session record when a backend starts (backend, version, node, TUN on/off) and an exit record when it stops (exit code or signal, whether it was requested, the crash count in the window, the last output line). Route-helper output reaches the same file through the process log stream.
- Log files and their directory are private to the user; the application's own records never contain subscription URLs or node credentials.
- Geodata refresh failures and subscription auto-update failures raise a toast once per failure streak.

## Capabilities

### New Capabilities

- `diagnostic-logs`: durable application and backend logs with bounded size, private permissions, and user-visible background failures.

### Modified Capabilities

_None._

## Impact

- New shared rotating-file writer in `crates/core`; logger initialization in `crates/ui/src/main.rs`.
- `crates/process/src/manager.rs` — backend reader tasks also write to the backend log file; session and exit records.
- `crates/ui/src/{geodata_service.rs,subscriptions.rs,app.rs}` — failure toasts.
- New files under `$XDG_STATE_HOME/v2ray-rs/logs/` (or `<data_dir>/state/logs/`), bounded to a few MiB per file. No new crate dependencies.
- Pairs with `harden-connection-lifecycle`, which routes route-helper output into the process log stream.
