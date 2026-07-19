# Design: TUN-session latency-probe gating

## Context

Upstream Xray-core panics on a nil `RemoteAddr` from gVisor when a TCP connection through the TUN closes between accept and handling (issue #6364; fixed only in 26.6.27, which no package currently ships). TCP pings are the app's own steady source of exactly that pattern — the scheduled refresh fires a probe volley every 10 minutes. Independently of the panic, probes captured by the tunnel measure a proxied path while the UI labels them direct.

## Goals / Non-Goals

- Goal: the app never self-triggers the upstream panic, on any xray version, and never records tunnel-skewed values as direct latency.
- Non-goal: routing probes around the tunnel (SO_MARK/SO_BINDTODEVICE need capabilities the GUI process does not hold; a bypass-uid re-exec of a probe helper is machinery disproportionate to "skip the tick").
- Non-goal: changing Real Delay — it already probes through the running proxy and is correct under TUN.

## Decisions

- Gate on "active connection has TUN enabled" (the app already tracks the runtime snapshot and connection state), not on backend type or version: sing-box TUN skews the measurements identically even though it does not panic.
- Scheduled tick: skip and log at debug; do not reschedule early. Stale-but-labeled-stale beats fresh-but-wrong.
- Manual button: insensitive with a tooltip/hint rather than a toast-on-click, matching how other unavailable actions are presented; "Test Real Delay" stays available.
- Sorting/auto-resolve keep consuming the persisted snapshot unchanged.

## Risks / Trade-offs

- [Latency data ages during long TUN sessions] → acceptable: values are ordering hints; Real Delay covers active-session quality, and disconnecting re-enables refresh.

## Migration Plan

UI-layer only; no data changes. Rollback = revert.

## Open Questions

None.
