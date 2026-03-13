# Design: Single Connections

## Context
The current connect path loads subscriptions, builds ordered candidates, and writes backend config from the selected node. Manual nodes should plug into that path as another candidate source, not as a synthetic subscription.

## Architecture

### 1. Manual node model and persistence
- Introduce `ManualNode { id: Uuid, node: ProxyNode, enabled: bool }`.
- Persist manual nodes in `custom_nodes.json` using the existing atomic persistence helpers.
- Reuse the existing `ProxyNode` protocol structs for forms and serialization.
- Latency remains in the shared latency snapshot store rather than on the persisted manual-node record.

### 2. Stable candidate identity
- Replace subscription-only candidate identity with a `ConnectionNodeRef` enum:
  - `Subscription { subscription_id, node_index }`
  - `Manual { node_id }`
- Use `ConnectionNodeRef` for planner output, last-success tracking, latency snapshot keys, and active connection metadata.
- `ConnectionPlanner` accepts both subscription nodes and manual nodes and preserves stable identity across insert and delete operations on manual nodes.

### 3. Connect path and config generation
- `AppMsg::Connect` loads subscriptions and manual nodes, passes both into candidate planning, and continues to call `ConfigWriter` with the selected `ProxyNode`.
- Manual-node add, edit, delete, and enable/disable changes regenerate config immediately only when disconnected.
- While connected, manual-node changes persist, set the restart-required state, and can be applied by `Apply & Restart` or reverted by `Discard`.

### 4. UI integration
- Add a `Nodes` section beside `Subscriptions` in the upper pane of the main window.
- Provide add and edit dialogs for VLESS, VMess, Shadowsocks, and Trojan using the existing protocol models.
- When a manual node is active, the status bar and tray source label is `Manual` and the node name comes from the manual-node entry.
- Reuse the existing restart-required banner so connected manual-node changes surface the same `Apply & Restart` and `Discard` actions as other runtime-relevant edits.
