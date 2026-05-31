# Tasks: Fix Real Delay disabled/stuck for Xray backend

## 1. Fix capability reset on binary path change

- [x] 1.1 In `SubscriptionsPage::update` handling `SyncSettings`, track `binary_changed` in addition to `backend_changed`. Reset `real_delay_capability` when either changes.
- [x] 1.2 Verify with a test or manual check that switching Xray binary paths resets the capability from `Unsupported` back to `PotentiallySupported`.

## 2. Add hard outer timeout and stale-result guard

- [x] 2.1 Store active Real Delay runs as `subscription_id -> run_token` in `SubscriptionsPage`.
- [x] 2.2 Increment the run token in `SyncSettings` when backend type or binary path changes, and clear stale active runs.
- [x] 2.3 In `TestRealDelay` handler, capture the current run token. Wrap the async command body in `tokio::time::timeout(duration, ...)`. Include the token in `RealDelayResult`.
- [x] 2.4 Extend `SubscriptionsCmdOutput::RealDelayResult` to include the token: `RealDelayResult(Uuid, u64, RealDelayReport)`.
- [x] 2.5 In `update_cmd` handler for `RealDelayResult`, discard the result unless the token matches the current active run. Only matching results clear the active run.
- [x] 2.6 On timeout, produce a failed `RealDelayReport` with a diagnostic like "Real Delay probe timed out".

## 3. Wire Real Delay preference controls

- [x] 3.1 In `preferences/network.rs`, connect `real_delay_enabled_row.connect_active_notify` to update `settings.real_delay.enabled` and call `emit`.
- [x] 3.2 Connect `real_delay_url_row.connect_apply` to update `settings.real_delay.test_url` and call `emit`. Validate the URL first using `AppSettings::validate_real_delay_url`.
- [x] 3.3 Connect `real_delay_timeout_row.connect_changed` to update `settings.real_delay.timeout_ms` and call `emit`.
- [x] 3.4 Connect `real_delay_use_for_lowest_row.connect_active_notify` to update `settings.real_delay.use_for_lowest_latency` and call `emit`.
- [x] 3.5 Connect `real_delay_preset_row.connect_selected_notify` to update the test URL from the selected preset and apply it to the URL row and settings.

## 4. Verification

- [x] 4.1 `cargo check --workspace` passes.
- [x] 4.2 `cargo test --workspace` passes.
- [x] 4.3 `cargo clippy --workspace -- -D warnings` passes.
- [ ] 4.4 Manual test: switch Xray binary paths in Preferences, verify Real Delay button resets to enabled.
- [ ] 4.5 Manual test: toggle Real Delay enabled in Preferences, verify it persists across app restart.
- [ ] 4.6 Manual test: run Real Delay with Xray, verify button returns to normal state after completion or timeout.
