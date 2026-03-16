# Proposal: Single Connections

## Why
The app only works with subscription-derived nodes. Users need manual nodes for ad hoc testing, but the feature must fit the existing connect path, persistence model, and status UI without pretending manual nodes are subscriptions.

## What Changes
- **First-class manual node source**: Add persisted manual nodes with stable IDs instead of modeling them as a fake subscription.
- **Stable connection identity**: Extend connection planning, last-success tracking, and latency history to reference a real node source identifier for manual nodes.
- **Persistent storage**: Store manual nodes in `custom_nodes.json` with the same atomic-write guarantees as other app data.
- **Main-window integration**: Add a `Nodes` section alongside `Subscriptions` in the upper pane and show `Manual` as the source label when a manual node is active.
- **Config generation and restart policy**: Disconnected manual-node changes regenerate config immediately; connected add, edit, delete, and enable/disable changes become pending until the user applies restart or discards them.

## Capabilities

### New Capabilities
- `single-connection-management`

### Modified Capabilities
- `app-persistence`
- `connection-auto-resolve`
- `config-generator`
- `main-window`
- `system-tray`
- `ui-chrome`
- `ui-lists`
