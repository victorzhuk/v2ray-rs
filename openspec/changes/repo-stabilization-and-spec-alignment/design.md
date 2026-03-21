# Design: Repo Stabilization and Spec Alignment

## Context
The repo already had the primitives needed for most of the missing behavior, but they were not wired together consistently. Subscription parsing already produced partial-success diagnostics, geodata settings already existed, and backend validation logic already knew about version probing, yet the UI either hid those results or bypassed them.

## Design Decisions

### 1. Backend availability stays visible
- Keep detected backends in the UI even when version probing fails.
- Treat version-probe failures as unavailable selections instead of silently degrading to `version = None`.
- Auto-select only when exactly one detected backend is actually usable.

### 2. Subscription import becomes non-destructive
- Route add/import flows through `SubscriptionService` instead of pre-persisting an empty subscription.
- Reuse parser partial-success output for diagnostics.
- Persist only after at least one valid node exists.
- Keep refresh reconciliation behavior and preserve existing stored data when an update yields no valid nodes.

### 3. Runtime state is explicit
- Pre-launch binary/config validation transitions through `Starting -> Error` so the process state contract is observable to the UI and tray.
- The logs view keeps the in-memory buffer visible while disconnected and overlays a non-running indicator instead of hiding the content.
- Dead connection-state persistence is removed rather than expanded into an unspecced recovery feature.

### 4. Geodata refresh is a small service
- Add a dedicated UI-side geodata refresh service that reacts to settings changes.
- On each pass, download only when geodata is missing or stale, then reindex when new files arrive or the index is missing.
- Failures log warnings and preserve the previous geodata/index state.

### 5. Tooling is pinned and honest
- Add `rust-toolchain.toml` so local `fmt` and `clippy` are reproducible.
- Make CI run the same all-target lint/test commands used in local validation.

