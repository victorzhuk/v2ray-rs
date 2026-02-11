# Proposal: Process Management

## Why

The app must manage the lifecycle of the selected CLI backend process — launching it with the generated config, monitoring its health, capturing logs/errors, and gracefully stopping or restarting it. Without reliable process management, the app cannot fulfill its core purpose of being a GUI wrapper.

## What Changes

- Launch the selected backend binary with the generated config file path as argument
- Gracefully stop the running process (SIGTERM, then SIGKILL after timeout)
- Restart the process when config is regenerated or user requests it
- Capture stdout/stderr and expose log output to the UI
- Monitor process health (detect crashes, unexpected exits)
- Report connection status (running, stopped, error) to the UI/tray
- Prevent zombie processes on app exit

## Capabilities

### New Capabilities
- `process-lifecycle`: Launch, stop, restart, and monitor v2ray/xray/sing-box backend processes with log capture

### Modified Capabilities
(none)

## Impact

- New module: `process`
- Dependencies: tokio::process (async process management), nix (signal handling)
- Depends on: backend-detection (binary path), config-generation (config file path)
- Feeds into: system-tray-integration and main-window-ui (status and log display)
