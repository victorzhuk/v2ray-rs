## Why

Node hostnames are pinned to IPs through the OS resolver on every connect, TUN or not, one candidate at a time, and the answers are frozen into the config for the whole session. On networks that intercept UDP/53 those answers can be the network's, not the resolver's; during failover a failed xray candidate stays parked with its routes installed, so the next candidate's lookup is captured by that tunnel and times out. When that happens DNS capture is silently turned off: the only trace is `WARN cannot pin ap4.directly.chat: lookup timed out` in the app log (2026-09-15 05:03:58). The same morning `backend.log` shows 306 `dial tcp <ip>:443: i/o timeout` errors for `ap2.directly.chat` spread over 11 different addresses.

## What Changes

- Node hostnames are pinned only for TUN connections; a non-TUN connection lets the backend resolve its server at dial time as before pinning existed.
- All hostnames of a connection attempt (every candidate and the nodes its rules route through) are resolved once, concurrently, before the first candidate starts, so no lookup runs while a parked candidate's routes are installed.
- Before a candidate starts, its pinned addresses are TCP-probed on the node's port with a short timeout; unreachable addresses are dropped, and all are kept when none answers or when a parked candidate's routes would make the probe meaningless.
- A hostname that cannot be resolved falls back to the addresses pinned by the last session that reached `Running`, when available.
- When xray DNS capture has to stay off because a node is unpinned, the user sees a toast, the process log gets a notice naming the host, and the session record states `capture_dns=false`.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `tun-mode`: "Proxy hostname resolution is bootstrapped" gains TUN-only scope, up-front resolution per attempt, reachability filtering, last-good fallback, and user-visible disabled capture.

## Impact

- `crates/ui/src/connection.rs` — pinning moved before the candidate loop, reachability probe, notices, `AppMsg` for last-good pins.
- `crates/ui/src/app.rs` — keeps last-good pins in memory and passes them with each `ConnectionRequest`; toast.
- `crates/process/src/manager.rs` — accessor reporting whether a manager holds a TUN runtime.
- No config generator, persistence, or dependency change.
