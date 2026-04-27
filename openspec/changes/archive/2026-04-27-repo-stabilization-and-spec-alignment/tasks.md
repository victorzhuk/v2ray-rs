## 1. Backend onboarding

- [x] 1.1 Keep detected-but-unavailable backends visible with validation/probe error details
- [x] 1.2 Auto-select the sole available backend in onboarding
- [x] 1.3 Add validated custom backend path controls in onboarding and preferences
- [x] 1.4 Migrate persisted `geo-aware` strategy values to `last-successful`

## 2. Subscription import and update

- [x] 2.1 Route add/import flows through `SubscriptionService`
- [x] 2.2 Support file-based imports in onboarding and the subscriptions page
- [x] 2.3 Surface partial parse failures to the UI
- [x] 2.4 Reject zero-valid-node imports and keep previous stored data on invalid refresh

## 3. Runtime and geodata behavior

- [x] 3.1 Transition process state to `Error` on missing binary/config before returning
- [x] 3.2 Keep stopped-state logs visible with an indicator
- [x] 3.3 Remove dead connection-state persistence
- [x] 3.4 Add background geodata refresh driven by app settings

## 4. Quality gates and spec alignment

- [x] 4.1 Pin the Rust toolchain
- [x] 4.2 Make CI run `clippy --all-targets -D warnings`
- [x] 4.3 Make CI run `cargo test --workspace --all-targets`
- [x] 4.4 Add delta specs for the affected capabilities

