## Why

The TUN capability preflight is incomplete: `getcap` runs with no timeout (a wedged probe hangs the connect) and a missing `getcap` surfaces as an opaque probe failure; a root session needlessly probes capabilities at all; the xray route helper is first exercised by `xray-up` after the backend is already spawned, so a helper that is missing, not yet executable (relocated copy), or lacks `CAP_NET_ADMIN` is discovered after a backend exists; a backend binary on a `nosuid` mount fails only at grant time. When a preflight does fail, the failure repeats on every candidate of the connection attempt and the user gets a plain `Error:` toast — the "Grant TUN privileges" action exists only in Preferences → TUN.

## What Changes

- Capability probes are bounded by a timeout, a missing `getcap` yields an error naming libcap, and a process running as root skips capability checks.
- xray TUN connect checks the route helper before spawn: it must resolve to an existing file this process can execute and must hold `CAP_NET_ADMIN`; a relocated helper the session cannot execute yet asks the user to log out and back in.
- A backend binary on a mount that ignores file capabilities fails the TUN connect with the manual `setcap` remedy instead of the grant action.
- Host-level preflight failures (capabilities, helper, mount, versions) end the connection attempt at once instead of repeating on every candidate, and a missing capability shows a toast with a "Grant TUN privileges" button that opens Preferences on the TUN page.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `tun-mode`: "TUN requires elevated capabilities granted once" covers the helper, probe bounds, root, `nosuid`, the attempt-level stop and the connect-time grant action.

## Impact

- `crates/process/src/privilege.rs` — bounded `getcap`, euid check.
- `crates/process/src/manager.rs` — helper and mount checks, host-level error classification (`ProcessError::is_host_level`), new error variants.
- `crates/ui/src/connection.rs` — stop the candidate loop on host-level errors; grant-needed signal.
- `crates/ui/src/app.rs` — grant toast action on connect.
- No config, persistence, or dependency change. Version gates and geodata are separate changes (`check-backend-versions-geodata`); recovery surfacing is `surface-tun-recovery-failures`.
