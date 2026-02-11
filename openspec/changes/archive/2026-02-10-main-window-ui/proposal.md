# Proposal: Main Window UI

## Why

The main window is the primary interface for managing subscriptions, editing routing rules, viewing logs, and configuring the app. It provides the full-featured experience beyond what the system tray offers. A modern, intuitive Relm4/GTK4 UI is essential for the target audience of Linux desktop users.

## What Changes

- Relm4/GTK4 application scaffolding with main window
- Tab/page-based layout: Subscriptions, Routing Rules, Logs, Settings
- Subscriptions page: list subscriptions, add/remove, view nodes, enable/disable nodes
- Routing Rules page: list rules, add/edit/remove, drag-and-drop reordering
- Logs page: live log viewer from backend process stdout/stderr
- Settings page: backend selection, proxy ports, auto-update intervals, appearance
- First-time onboarding wizard (import subscription, select backend)
- Connection status bar with connect/disconnect button
- Internationalization-ready structure (English, Russian)

## Capabilities

### New Capabilities
- `main-window`: Relm4/GTK4 main application window with tabbed layout for subscription, routing, log, and settings management
- `onboarding-wizard`: First-time setup wizard for backend selection and initial subscription import

### Modified Capabilities
(none)

## Impact

- New module: `ui` with per-page components
- Dependencies: relm4, gtk4, libadwaita (for modern GNOME look)
- Depends on: all other changes (subscription-management, routing-rule-manager, process-management, backend-detection, system-tray-integration)
- This is the top-level integration point
