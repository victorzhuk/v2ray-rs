# Design: Fix Real Delay disabled/stuck for Xray backend

## Context

The Real Delay feature was extended to support Xray and V2ray through their ObservatoryService gRPC API in a previous change. However, the UI state management has gaps that cause the Real Delay button to remain disabled or stuck in "Testing" state.

The code flow is:
1. User clicks "Test Real Delay" → `SubscriptionsMsg::TestRealDelay(id)` → inserts `id` into `testing_real_delay` HashSet → spawns oneshot_command
2. Async command runs `measure_real_delay()` → returns `RealDelayReport`
3. `SubscriptionsCmdOutput::RealDelayResult(id, report)` arrives → removes `id` from `testing_real_delay`
4. UI re-renders with updated state

The button sensitivity is computed as:
```rust
.sensitive(!is_testing_real_delay && real_delay_available && btn_sensitive)
```

Where `real_delay_available = real_delay_settings.enabled && backend_type.supports_real_delay()` and `btn_sensitive` depends on `real_delay_capability`.

## Goals / Non-Goals

**Goals:**
- Ensure Real Delay capability resets when backend type or binary path changes.
- Guarantee `testing_real_delay` is always cleared, even if the async command hangs.
- Discard stale results after mid-probe backend changes.
- Wire Real Delay preference controls so changes actually persist.

**Non-Goals:**
- Changing the probe config generation.
- Changing the observatory gRPC client.
- Adding proactive capability detection at startup.

## Decisions

### D1. Track binary_path change in SyncSettings

Current code at `subscriptions.rs:683` only checks `backend_changed = self.backend_type != backend_type`. Extend this to also check `binary_changed = self.binary_path != binary_path` and reset `real_delay_capability` when either changes.

### D2. Add a hard outer timeout to the Real Delay command

The current async command has no overall timeout. The observatory polling loop has a deadline, but `ProbeRunner::start()` or `ProbeRunner::stop()` could hang. Wrap the entire oneshot_command body in `tokio::time::timeout(Duration::from_millis(timeout_ms + 15000), ...)`. On timeout, return a failed `RealDelayReport` so the UI always clears.

### D3. Track active probes with run tokens

Store active Real Delay probes as `subscription_id -> run_token`. Increment the token whenever a probe starts and whenever the backend type or binary path changes. Capture the token when spawning the command. On `RealDelayResult` arrival, apply and clear the result only if the token still matches the active run for that subscription.

This avoids a stale result from an old backend clearing a newer in-flight probe for the same subscription.

### D4. Wire Real Delay preference rows

In `preferences/network.rs`, connect:
- `real_delay_enabled_row` → `connect_active_notify` → update `settings.real_delay.enabled`
- `real_delay_url_row` → `connect_apply` → update `settings.real_delay.test_url`
- `real_delay_timeout_row` → `connect_changed` → update `settings.real_delay.timeout_ms`
- `real_delay_use_for_lowest_row` → `connect_active_notify` → update `settings.real_delay.use_for_lowest_latency`

Each handler calls `emit(&st, &cb)` to trigger `SettingsChanged` → `FlushSettings`.

## Risks / Trade-offs

- **Generation counter overflow**: Practically impossible with `u64`. Wrapping add is fine.
- **Hard timeout too aggressive**: 15 seconds above the configured timeout gives ample margin for process shutdown. If needed, users can increase `timeout_ms`.
- **Preference wiring regression**: The rows already exist; adding signal handlers is additive. No existing behavior changes.

## Migration Plan

No data migration needed. This is a pure bug fix affecting runtime UI state.
