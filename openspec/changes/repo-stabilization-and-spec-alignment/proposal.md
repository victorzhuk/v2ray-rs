# Proposal: Repo Stabilization and Spec Alignment

## Why
Several active specs no longer match shipped behavior, and some critical user-visible flows were still incomplete even though the codebase already contained most of the underlying pieces. The largest gaps were backend onboarding, subscription import diagnostics, stopped-state logs, and unused geodata refresh settings. At the same time, local quality gates had drifted from CI because the toolchain was unpinned and CI did not lint or test all targets.

## What Changes
- Align backend detection UX with the promised contract: single-backend auto-select, visible unavailable backends, and validated custom binary paths.
- Make subscription import/update source-aware and diagnostic-rich: support file sources in UI, surface invalid URI skips, and refuse zero-valid-node imports.
- Stabilize runtime behavior: missing binary/config now transitions to `Error`, stopped-state logs remain visible, and dead connection-state persistence is removed.
- Activate background geodata refresh using the existing settings model and keep the manual refresh path intact.
- Pin the Rust toolchain and make CI run the same `fmt`, `clippy --all-targets`, and `test --all-targets` commands used locally.

## Capabilities

### New Capabilities
- None

### Modified Capabilities
- `backend-detection`
- `subscription-import`
- `subscription-update`
- `geodata-management`
- `process-lifecycle`
- `main-window`
- `connection-auto-resolve`
- `routing-rules`
- `system-tray`
- `ui-drag-and-drop`

