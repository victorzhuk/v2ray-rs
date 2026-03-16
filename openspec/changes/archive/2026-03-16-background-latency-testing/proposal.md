# Proposal: Background Latency Testing

## Why
Users can manually test latency for a subscription, but the app does not refresh latency in the background while connected. Lowest-latency ordering therefore depends on stale samples, and users cannot refresh latency without opening the subscription menu. This change should extend the existing direct-TCP latency path instead of introducing a second process-coupled system.

## What Changes
- **Keep direct TCP testing**: Reuse `crates/subscription/src/ping.rs` and keep latency testing decoupled from backend process control.
- **Session-local scheduler**: Add a fixed 10-minute background refresh loop in `SubscriptionsPage` for enabled nodes in enabled subscriptions while the app is running. The 10-minute interval balances freshness with network overhead for typical VPN usage patterns.
- **Shared latency pipeline**: Make manual "Test Latency" and scheduled refreshes use the same result path, updating `last_latency_ms` and `latency_snapshot.json`.
- **Incremental UI updates**: Update node rows as latency results arrive without disconnecting or restarting the backend.

## Non-Goals
- Measuring "real-world" latency through the local SOCKS/HTTP inbound
- Moving latency ownership into `App` or `ProcessManager`
- Background latency refresh for manual (single-connection) nodes — manual nodes do not participate in scheduled latency testing

## Capabilities

### New Capabilities
- `background-latency-testing`

### Modified Capabilities
- `ui-lists`
