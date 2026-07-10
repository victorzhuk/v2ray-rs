## 1. App plumbing

- [ ] 1.1 Add `AppMsg::ConnectToNode(ConnectionNodeRef)`; handler resolves the node from current subscriptions/manual nodes into a single `ConnectionCandidate` and dispatches the existing connection spawn with `vec![candidate]`
- [ ] 1.2 Already-connected path: record the pending target, dispatch Disconnect, and connect to the target when the stop completes (extend the existing `reconnect_pending` consumption)
- [ ] 1.3 Guard: node missing or disabled at dispatch time → toast, no state change

## 2. Manual nodes UI

- [ ] 2.1 Add Connect to the per-row popover in `nodes.rs`, insensitive/hidden when the node is disabled; new `NodesOutput::ConnectNode(node_id)` forwarded to the app

## 3. Subscription nodes UI

- [ ] 3.1 Add a per-node Connect affordance in `subscriptions.rs` `build_node_row` (respect the two-suffix-widget rule), insensitive/hidden when disabled; new `SubscriptionsOutput::ConnectNode(sub_id, node_id)` forwarded to the app

## 4. Tests & verification

- [ ] 4.1 Unit test for the node-ref → single-candidate resolution (enabled, disabled, missing)
- [ ] 4.2 `cargo test --workspace` green; `cargo clippy` clean; manual run: direct connect while disconnected and while connected
