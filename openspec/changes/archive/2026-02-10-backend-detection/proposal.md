# Proposal: Backend Detection & Selection

## Why

The app wraps system-installed v2ray, xray, and sing-box binaries. It must detect which backends are available, allow users to select one, and gracefully handle missing binaries with actionable guidance. This is a prerequisite for config generation and process management.

## What Changes

- Auto-detect presence of `/usr/bin/v2ray`, `/usr/bin/xray`, `/usr/bin/sing-box` (and common alternative paths) on startup
- Query binary version via `--version` flag
- Allow user selection when multiple backends are installed
- Provide clear error messages and installation guidance when no backend is found
- Store selected backend preference persistently

## Capabilities

### New Capabilities
- `backend-detection`: Detect, validate, and select v2ray/xray/sing-box binaries on the system

### Modified Capabilities
(none)

## Impact

- New module: `backend` or `detector`
- Dependencies: std::process::Command, which crate (optional)
- Depends on: core-data-models (for backend config model)
- Affects: config-generation, process-management (they consume the selected backend)
