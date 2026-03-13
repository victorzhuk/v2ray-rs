## 1. Data model and persistence

- [x] 1.1 Add the manual-node model and `custom_nodes.json` persistence helpers
- [x] 1.2 Extend last-success tracking, latency snapshot keys, and active connection metadata to use `ConnectionNodeRef`
- [x] 1.3 Add round-trip and corrupt-file verification for manual-node persistence

## 2. Connect path and config generation

- [x] 2.1 Load manual nodes in `AppMsg::Connect` and include them in candidate planning
- [x] 2.2 Keep `ConfigWriter` input as the selected `ProxyNode`
- [x] 2.3 Apply the restart-required and discard-restore policy to connected manual-node changes

## 3. UI implementation

- [x] 3.1 Add a `Nodes` section alongside `Subscriptions` in the upper pane
- [x] 3.2 Implement boxed-list rows, enable/disable toggles, and grouped secondary actions for manual nodes
- [x] 3.3 Create add and edit dialogs for VLESS, VMess, Shadowsocks, and Trojan
- [x] 3.4 Show `Manual` as the source label in the status bar and tray when a manual node is active
- [x] 3.5 Show the restart-required banner for connected manual-node changes and restore launched manual nodes on `Discard`

## 4. Verification

- [x] 4.1 Verify last-success and latency history stay attached to the same manual node after insert and delete operations
- [x] 4.2 Verify add, edit, delete, and enable/disable state persist across restart
- [x] 4.3 Verify `Discard` restores the launched manual-node set while connected
