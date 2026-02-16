## 1. Settings and Models

- [x] 1.1 Add auto-resolve strategy enum and connection metadata structs in core models
- [x] 1.2 Persist auto-resolve strategy and last-success metadata in AppSettings
- [x] 1.3 Add helper to load/save connection metadata and latency snapshots

## 2. Connection Planning

- [x] 2.1 Implement connection candidate planner (list order, latency, random, last-success, geo-aware)
- [x] 2.2 Add latency cache and ordering rules for missing latency values
- [x] 2.3 Add tests for candidate ordering strategies

## 3. Config + Process Orchestration

- [x] 3.1 Update config generation to accept single active candidate per attempt
- [x] 3.2 Add sequential connection attempts with per-candidate config regeneration
- [x] 3.3 Emit process events with connection metadata (active candidate, strategy, since)

## 4. UI Status Panel

- [x] 4.1 Extend main window status panel layout with connection details
- [x] 4.2 Wire status panel to process events and connection metadata updates
- [x] 4.3 Add empty/disconnected placeholder text for connection details

## 5. Tray Tooltip and Menu

- [x] 5.1 Extend tray tooltip to display connection metadata details
- [x] 5.2 Update tray menu status label to include active node name

## 6. Preferences UI

- [x] 6.1 Add global auto-resolve strategy selector to preferences
- [x] 6.2 Persist selection and trigger reconnect when strategy changes
