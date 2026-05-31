# Proposal: Fix Real Delay disabled/stuck for Xray backend

## Why

The Real Delay test button appears permanently disabled or stuck in "Testing Real Delay..." state when using the Xray backend. The investigation found four root causes:

1. **Binary-path change does not reset capability**: `SyncSettings` only resets `real_delay_capability` when the backend type changes, not when the binary path changes. This violates the spec requirement in `add-observatory-real-delay/specs/backend-detection/spec.md:24-27`. If one Xray binary was marked unsupported, switching to another Xray binary leaves Real Delay disabled.

2. **No hard timeout on the async Real Delay command**: If the Xray observatory query or shutdown path hangs, the subscription ID stays in `testing_real_delay` forever, leaving the menu stuck as "Testing Real Delay...".

3. **Stale results after backend change**: There is no run token to discard stale `RealDelayResult` messages that arrive after the user switches backends mid-probe.

4. **Real Delay preferences not wired**: The Real Delay SwitchRow, EntryRow, SpinRow, and "Use for Lowest Latency" SwitchRow in Preferences are created but never connected to update `AppSettings.real_delay`. If `real_delay.enabled` is persisted as false, the UI switch may not actually re-enable it.

## What Changes

- Reset `real_delay_capability` when either `backend_type` or `binary_path` changes in `SyncSettings`.
- Add a hard outer timeout (e.g., `timeout_ms + 15s`) around the entire Real Delay command so `testing_real_delay` is always cleared.
- Track active Real Delay runs with per-run tokens; discard stale `RealDelayResult` messages without clearing newer in-flight runs.
- Wire all Real Delay preference rows (enabled, test URL, timeout, use-for-lowest) to update and persist `AppSettings.real_delay`.

## Capabilities

### Modified Capabilities
- `backend-detection`: capability reset now triggers on binary-path change, not just type change.
- `real-delay-latency-test`: Real Delay probe has a hard timeout; stale results are discarded after backend change.

## Impact

- **Crates affected**:
  - `v2ray-rs-ui`: `subscriptions.rs` (SyncSettings, TestRealDelay, RealDelayResult handler), `preferences/network.rs` (wire Real Delay rows).
- **External behavior**: Real Delay button no longer gets stuck; changing the Xray binary resets capability; preference toggles actually persist.
- **Risk**: Low. Changes are localized to UI state management and preference wiring.
