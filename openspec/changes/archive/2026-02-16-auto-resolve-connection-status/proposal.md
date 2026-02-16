## Why

Connection behavior is currently ambiguous when multiple enabled nodes exist, and the UI provides limited visibility into which node is active or why connection succeeds/fails. Users need predictable auto-selection and clear, always-visible status to understand which node is in use and how the connection was established.

## What Changes

- Add a global auto-resolve strategy setting (list order, lowest latency, random, last successful, geo-aware) that governs how enabled nodes are selected for connection attempts.
- Build an ordered connection candidate list from subscriptions and nodes based on the selected strategy and persist/track the selected active node.
- Attempt connections in order and stop on first successful connection; report failures if no candidates connect.
- Extend main window with a bottom status panel showing connection state and active node details.
- Extend tray tooltip to mirror connection status and active node details (subscription/node/latency/backend/strategy/since).

## Capabilities

### New Capabilities
- `connection-auto-resolve`: Select, order, and attempt connection candidates using a configurable strategy, including latency and last-success tracking.

### Modified Capabilities
- `main-window`: expand connection status area to include active node details and strategy.
- `system-tray`: expose detailed connection status in tooltip and menu metadata.
- `ui-statusbar-logs`: adjust status bar requirements to support the expanded status panel layout.
- `config-generator`: update multiple-node handling to align with ordered selection and per-connection candidate metadata.
- `process-lifecycle`: reflect connection attempt outcomes and report active node context with state events.

## Impact

- Settings model and persistence to store global auto-resolve strategy and metadata (last successful node, last latency).
- Subscription/connection orchestration in UI and/or core to build ordered candidates and manage sequential attempts.
- Tray and main window UI layout changes to include status panel and tooltip content.
- Config generation and process management updates to support per-attempt configs and surfaced active node context.
