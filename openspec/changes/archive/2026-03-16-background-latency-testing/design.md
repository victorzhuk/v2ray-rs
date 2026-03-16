# Design: Background Latency Testing

## Context
Latency testing already uses direct TCP connects and runs outside `ProcessManager`. The missing piece is a reusable scheduling and persistence path for periodic refreshes.

## Architecture

### 1. Reuse the current ping pipeline
- Keep `ping_nodes` as the batched latency executor for a slice of `SubscriptionNode`.
- Keep latency execution and result ownership in `SubscriptionsPage`; `App` does not own subscription or node state.
- Continue storing durable samples in `latency_snapshot.json` and mirroring the latest sample into in-memory `last_latency_ms` for rendering.

### 2. Session-local background refresh
- `SubscriptionsPage` owns a session-local timer that fires every 10 minutes while the app is running.
- Each tick selects enabled nodes from enabled subscriptions that are not already under test and dispatches the existing async latency command path.
- Manual "Test Latency" uses the same pipeline and skips duplicate in-flight work for the same subscription.

### 3. Result handling
- When a latency command completes, `SubscriptionsPage` updates the targeted subscription nodes, persists the latency snapshot, and re-renders only the affected list state.
- The running backend process is not stopped, restarted, or consulted.

## Data Flow
`BackgroundLatencyTick → TestLatency(id) inputs → oneshot_command(ping_nodes) → LatencyResult → snapshot persist + row refresh`

Manual path: `user action → TestLatency(id) → oneshot_command(ping_nodes) → LatencyResult → snapshot persist + row refresh`

## Known Limitations

- **Snapshot keying by index**: Snapshot entries are keyed by `(subscription_id, node_index)`. Reordering nodes or refreshing a subscription may shift indices, causing snapshot entries to point to wrong nodes. A future change could key by a stable identifier (e.g., hash of address+port+protocol).
- **Timer fires with zero subscriptions**: The background timer reschedules regardless of whether any eligible subscriptions exist. This is harmless since `subscriptions_eligible_for_latency_test()` returns empty when there are no eligible targets.

## Non-Goals
- Measuring "real-world" latency through the local SOCKS/HTTP inbound
- Moving latency state into `App` or `ProcessManager`
