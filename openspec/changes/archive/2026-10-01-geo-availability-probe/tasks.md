## 1. Service check model

Prerequisite: show-node-egress (per-node loopback inbounds).

- [x] 1.1 Add `crates/core/src/geo_probe.rs`: `ServiceCheck {id, name, url, blocked_url_markers, blocked_body_markers}` with the NotebookLM and Gemini built-ins, and pure `derive_verdict(final_url, status, body) -> Verdict`; unit tests for every scenario in the built-in checks requirement; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [x] 1.2 Validate built-in URLs as `https://` at construction; unit test rejects a non-https check URL

## 2. Runner

- [x] 2.1 In `crates/subscription/src/real_delay.rs`, add the availability pass per node: for each service, `GET` through the node's loopback inbound with redirect cap 5, 2 MiB body cap, browser `User-Agent`, and the configured probe timeout; unit tests against a local stub server for redirect-to-blocked-marker, sign-in pass, refusal body, and timeout-as-unknown; `timeout 5m cargo test -p v2ray-rs-subscription -- --test-threads=4` green
- [x] 2.2 Honor the single-session rule: an availability request during a running session queues or toasts, and never spawns a second ephemeral backend; reuse the existing session guard; unit test with the stub runner

## 3. Persistence

- [x] 3.1 Add per-node per-service `verdicts` (`result`, `checked_at`) to the latency snapshot model in `crates/core/src/resolve.rs`; atomic write through the existing persist path; roundtrip unit test; `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` green
- [x] 3.2 Hydrate verdicts at startup with original checked-at times; unit test from a fixture snapshot

## 4. UI

- [x] 4.1 "Check availability" action on node selections in `crates/ui/src/subscriptions.rs`, gated by the same backend-support rules as Real Delay
- [x] 4.2 Results dialog: nodes × services badge grid with checked-at times; `blocked` badge shows the node's egress sample when present; `unknown` styled distinct from both other verdicts; manual widget-state unit test for the three badge styles and the egress hint

## 5. Verification

- [x] 5.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 5.2 Live xray session: probe nodes from two vpndio subscriptions; the Hetzner-exit node shows NotebookLM `blocked` and the AT-exit node `available`, matching manual `curl -L` verification; the active connection is untouched throughout
