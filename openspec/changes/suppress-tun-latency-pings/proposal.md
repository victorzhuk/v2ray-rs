# Suppress raw TCP latency probes while a TUN session is active

## Why

TCP latency probes are connect-then-immediately-close. With a TUN session active they are doubly wrong:

1. They no longer measure what they claim. An unmarked probe socket is captured by the tunnel and dialed through the proxy, so "direct TCP latency" silently becomes client → proxy → node round-trips.
2. On the affected Xray-core range (26.1.13 through 26.6.22 — every currently packaged build, including the installed 26.3.27) a quickly-closed connection through the TUN races gVisor's accept path into `panic: Net: Unknown address type.` (`common/net/destination.go:25` via `proxy/tun/handler.go`; upstream issue #6364, fixed in 26.6.27). Each panic drops the tunnel for the crash-restart window and resets every in-flight stream. The app's own 10-minute scheduled refresh and the manual "Test Latency" button are therefore a built-in periodic crash trigger — observed live as intermittent `panic: Net: Unknown address type.` in the process logs, followed by an automatic backend restart.

## What Changes

- The session-local 10-minute scheduled TCP refresh SHALL skip its tick while a TUN connection is active (any backend); persisted/hydrated samples continue to display.
- The manual "Test Latency" action SHALL be unavailable during an active TUN session, with a hint pointing at "Test Real Delay" — which probes through the running proxy and remains the honest metric under TUN.
- Startup hydration is unaffected.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `background-latency-testing`: "Direct TCP latency refresh while connected" and "Session-local scheduled latency refresh" gain TUN-session gating.

## Impact

- `crates/ui/src/app.rs` / `subscriptions.rs` — scheduled tick guard and Test Latency sensitivity/hint while the active connection has TUN enabled.
- No changes to `crates/subscription/src/ping.rs` itself.
- Related but separate: the TUN preflight version advisory for the same upstream panic ships with `harden-tun-dns-resolution`.
