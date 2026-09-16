## Why

A backend is reported `Running` as soon as its process is spawned, before it has proven it can serve traffic. sing-box accepts configs in `check` that then FATAL at start: on 2026-09-15 every sing-box TUN session exited about 100 ms after spawn, yet the UI flashed Connected, the node was saved as the last successful one, and the manager burned its crash budget with respawns 2 s and 4 s later on each candidate. A backend that never came up is a startup failure, not a crash of a working session.

## What Changes

- A start is reported `Running` only after the backend is ready: its local proxy inbound accepts TCP connections and the process is still alive one second after spawn. For xray TUN, readiness is checked after the TUN device and routes are up.
- A backend that exits, or does not become ready within the readiness timeout, during the initial start fails that start with the exit reason. It is not respawned and records no crash; the connection moves on to the next candidate (subject to `stop-failover-on-shared-failure`).
- An in-place respawn after a crash waits for readiness the same way; a respawn that exits before ready counts as a crash, as a failed respawn does today.
- The last-success record is written only when a connection reaches `Running` after readiness, never for `Starting` or respawn transitions.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `process-lifecycle`: new requirement that `Running` follows readiness; "Crash detection and recovery" limited to backends that were ready.
- `connection-auto-resolve`: "Last-success metadata is owned by connection outcomes" defines success as reaching ready `Running`.

## Impact

- `crates/process/src/manager.rs` — readiness wait in start and respawn, startup-failure error, no crash handling before ready.
- `crates/ui/src/connection.rs` — passes the inbound address to the manager; stub-backend tests listen on the inbound port.
- `crates/ui/src/app.rs` — last-success persisted only on `Running`.
- No config, persistence, or dependency change.
