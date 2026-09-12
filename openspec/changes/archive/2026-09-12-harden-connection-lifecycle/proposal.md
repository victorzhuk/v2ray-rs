# Keep TUN sessions closed and controllable across backend crashes

OVERSIZE: kill-switch spans helper, manager and app; halves ship a broken crash path

## Why

Between 2026-09-01 and 2026-09-11 the live install recreated its xray TUN device 176 times; 156 of those gaps were exactly 2.2s — the crash-restart delay — so the backend was dying and being respawned 8–28 times a day (xray 26.3.27, upstream TUN panic #6364, since upgraded to 26.9.9). The upgrade removes that trigger, but the crash path it exercised is broken in ways any future backend crash will hit again:

- Every respawn tears the routing state down before the restart delay, so for 2s plus the whole preflight the host's traffic and DNS leave directly through the ISP — unproxied, and on the networks this app serves, answered by a poisoning resolver whose answers applications then cache.
- A Disconnect that lands while a respawn is in flight is a no-op: `stop()` returns without a transition, no terminal state is ever reported, and the window sits on "Disconnecting…" forever.
- When a candidate exhausts its crash budget and the connection fails over, the dying manager's `Stopped` reaches the app with the live generation. The app drops its handle and clears the TUN recovery marker while the next candidate comes up; the UI then shows Connected with no way to disconnect short of quitting.
- A failed respawn (config-check timeout after resume, device timeout, helper failure) is not retried; it goes straight to failover.
- The route helper runs without a timeout, with its output going nowhere; a hung helper wedges every later connect behind the lifecycle lock.
- A connect in progress cannot be cancelled from the window or the tray.
- Under xray TUN with no IPv6 tunnel address, IPv6 is never routed into the tunnel, so dual-stack hosts leak it; `strict_route` exists in settings but only sing-box reads it.

## What Changes

- **Kill-switch while reconnecting (xray TUN, `strict_route` on).** The route helper installs a lowest-priority `unreachable` default route in the xray table for each family, so traffic that would enter the tunnel is refused whenever the tunnel device is gone instead of falling through to the real default route. Routing state is no longer torn down between a crash and its respawn, across candidate failover, or across the app's automatic reconnect attempts. It is released on Disconnect, on Quit, and on the final give-up once automatic reconnects are exhausted.
- **IPv6 under strict route.** With `strict_route` on and no IPv6 tunnel address, the helper installs the IPv6 policy rules and the IPv6 `unreachable` default, so IPv6 traffic that would leave through the real default route is refused; on-link IPv6 routes stay reachable. **BREAKING** for users on dual-stack networks who relied on IPv6 bypassing the xray tunnel: set an IPv6 tunnel address or turn `strict_route` off.
- `strict_route` applies to xray. Off keeps today's behavior for both points above.
- Stop is valid from every process state and always ends in a reported `Stopped`, including during a respawn, during start, and while a route-helper call is in flight. A teardown cut short by Quit completes before the app exits.
- Only the supervising connection reports terminal states; a candidate given up during failover is never reported as the connection stopping.
- A failed respawn counts against the crash budget and is retried until the budget is spent. An in-place respawn reuses the preflight results of the start it replaces instead of re-running the version probe, capability probe and config check.
- Route-helper invocations are bounded by a timeout, killed if their caller is cancelled, and their output reaches the process log stream; a failed teardown is reported instead of discarded.
- The TUN recovery marker is written before routes are programmed, from the interface the session actually uses.
- Disconnect is available while a connection is starting, in the window and the tray, and cancels the attempt.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `process-lifecycle`: "Stop backend process", "Crash detection and recovery" and "TUN-aware connection start and stop" change; new requirements for single-source terminal state reporting and cancelling an in-flight connection.
- `tun-mode`: "Privileged route helper for xray" gains the strict fallback routes and IPv6 handling; new requirement for blocking traffic while a session reconnects.
- `system-tray`: "Tray context menu" — Disconnect is enabled while starting.
- `ui-statusbar-logs`: "Connect button has icon and label" — the button stays actionable while starting.

## Impact

- `crates/process/src/{manager.rs,state.rs,tun.rs}` — stop from any state, respawn loop, preflight reuse, bounded and captured helper calls, no teardown on respawn or give-up.
- `crates/netctl/src/{main.rs,net.rs}` — `--strict` on `xray-up`: fallback `unreachable` routes and IPv6 rules without an IPv6 address; teardown and recovery remove them.
- `crates/ui/src/{connection.rs,app.rs}` — terminal-state ownership, kill-switch release on final give-up / Disconnect / Quit, marker written from the runtime, cancel during Starting.
- `crates/tray/src/tray.rs` — menu item enabled while starting.
- Behavior change: under xray TUN with `strict_route` on, the host has no connectivity outside the tunnel while a session is reconnecting, and no IPv6 default route unless an IPv6 tunnel address is set.
- sing-box sessions: unchanged except for the stop/terminal-state fixes.
