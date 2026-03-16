## 1. Runtime snapshot and divergence tracking

- [x] 1.1 Add a runtime snapshot type that contains only restart-relevant settings plus routing rules
- [x] 1.2 Capture the snapshot from the exact inputs passed into config generation during `AppMsg::Connect`
- [x] 1.3 Extend Preferences outputs or callbacks so routing-rule mutations reach `App`
- [x] 1.4 Compute `restart_required` only while the backend is starting or running

## 2. Banner behavior

- [x] 2.1 Add an `adw::Banner` to the main window for runtime divergence
- [x] 2.2 Implement `Apply & Restart` by reusing the existing disconnect/reconnect path
- [x] 2.3 Implement `Discard` by restoring persisted settings and routing rules from the active snapshot and closing any open Preferences dialog
- [x] 2.4 Clear the banner on apply, discard, disconnect, or failed connection start

## 3. Config regeneration policy

- [x] 3.1 Update config regeneration behavior so connected edits wait for apply or reconnect
- [x] 3.2 Keep disconnected edits regenerating config immediately
- [x] 3.3 Verify backend, port, DNS, as well routing changes while connected
