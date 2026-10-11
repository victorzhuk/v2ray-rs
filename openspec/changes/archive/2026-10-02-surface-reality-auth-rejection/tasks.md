## 1. Classification

- [x] 1.1 Extend `crates/ui/src/backend_warning.rs` with a classification table: `(backend, signature) -> ClassifiedFailure {cause, advice}`, starting with the xray REALITY `received real certificate` signature; pure match function over captured output; unit tests for match, no-match, and dedup-within-one-output; `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4` green
- [x] 1.2 Wire classification into the candidate-failure path in `crates/ui/src/connection.rs`: report replaces the generic failure text only when a signature matched; unit test with a stub connection run for classified, unmatched, and once-per-candidate cases

## 2. Refresh offer

- [x] 2.1 Attach a "Refresh subscription" action to the classified report for subscription-owned nodes, opening the existing manual update flow for that subscription; at most one offer per connection per subscription; unit test for the dedup and the manual-node no-offer case; `timeout 5m cargo test -p v2ray-rs-ui -- --test-threads=4` green
- [x] 2.2 No automatic update: assert through the update-manager stub that no update starts unless the action is activated; unit test

## 3. Verification

- [x] 3.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 3.2 Live: connect to a node whose server rejects REALITY auth (vpndio `ap4` pool while broken, or a lab server with rotated keys); the failure report shows the classified cause with the refresh action, exactly once, and failover to the next candidate behaves as before
