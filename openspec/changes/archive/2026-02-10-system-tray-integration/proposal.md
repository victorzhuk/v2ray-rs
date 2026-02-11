# Proposal: System Tray Integration

## Why

Linux desktop users expect tray-resident apps for VPN/proxy tools. The system tray provides quick connect/disconnect, profile switching, and status visibility without opening the main window. This is critical for daily-use convenience.

## What Changes

- Add system tray icon using libappindicator or ksni (Linux tray protocol)
- Show connection status via tray icon state (connected/disconnected/error with distinct icons)
- Tray menu: connect/disconnect toggle, active profile name, switch profile submenu, open main window, quit
- Minimize-to-tray on window close (configurable)
- Tray notifications for connection events (connect, disconnect, error)

## Capabilities

### New Capabilities
- `system-tray`: System tray icon with status display, quick actions menu, and notifications for proxy connection state

### Modified Capabilities
(none)

## Impact

- New module: `tray`
- Dependencies: ksni or libappindicator-rs, notify-rust (notifications)
- Depends on: process-management (connection status), core-data-models (profiles)
- Integrates with: main-window-ui (show/hide main window)
