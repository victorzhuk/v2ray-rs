## Why

A failed TUN route recovery at startup is invisible: `recover_tun_session` runs `netctl recover` with inherited stdio, so the helper's output goes to the app's stderr, the failure is only `log::warn!`-ed, the marker is deleted anyway, and stale routes remain with nothing on screen. The recovery pass is also bounded by `RECOVER_TIMEOUT` (5 s, `app.rs:37`) while every other route-helper invocation uses `HELPER_TIMEOUT` (10 s, `tun.rs:15`) — the mismatch is exactly what `docs/ARCHITECTURE.md` cannot document cleanly.

## What Changes

- `recover_tun_session` pipes the helper's stdout/stderr, writes each line and the outcome (`recover ok`, `recover exited with <status>`, `recover timed out after 10s`) to `backend.log` as `helper` records, returns the failure, and still clears the marker.
- Startup and release callers surface a failed recovery as a toast naming the manual command: `TUN route recovery failed: run v2ray-rs-netctl recover <--xray|--singbox> --iface <iface>`.
- `RECOVER_TIMEOUT` is replaced by `HELPER_TIMEOUT`, exported from the process crate, so the recovery pass uses the same 10 s bound as every other route-helper call.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `process-lifecycle`: new requirement that route-recovery failures are visible to the user, bounded by the route-helper timeout.
- `diagnostic-logs`: "Backend output survives the application" adds the recovery helper run to the backend log.

## Impact

- `crates/process/src/lib.rs` — `HELPER_TIMEOUT` export.
- `crates/ui/src/app.rs` — `recover_tun_session` output capture and logging, recovery toast, `RECOVER_TIMEOUT` removal.
- Capability gates and version/geodata preflight are separate changes (`gate-tun-capability-preflight`, `check-backend-versions-geodata`). No config, persistence, or dependency change.
