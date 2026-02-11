# Design: System Tray Integration

## Context

Linux system tray support varies by desktop environment. GNOME uses AppIndicator (libappindicator), KDE uses StatusNotifierItem (SNI), and others may use the legacy XEmbed protocol. The app needs to work across major DEs.

## Goals / Non-Goals

**Goals:**
- Tray icon visible on GNOME, KDE, XFCE, and other Linux DEs
- Connection state reflected in icon appearance
- Context menu with quick actions
- Desktop notifications for state changes

**Non-Goals:**
- No Windows/macOS tray support
- No animated tray icons

## Decisions

1. **Tray library**: Use `ksni` crate which implements the StatusNotifierItem D-Bus protocol. Falls back gracefully. Most modern Linux DEs support SNI.

2. **Icon states**: Three icon states — disconnected (gray), connected (green/colored), error (red). Use SVG icons embedded in binary via `include_bytes!`.

3. **Menu structure**: Connect/Disconnect toggle → separator → Active profile name (non-clickable) → Profile submenu → separator → Open Main Window → Quit.

4. **Notifications**: Use `notify-rust` crate for desktop notifications. Notify on: connection established, connection lost, error states. Configurable (can be disabled).

## Risks / Trade-offs

- [DE compatibility] Some DEs may not support SNI → Provide fallback or document limitation; GNOME requires AppIndicator extension
- [ksni maturity] ksni is a smaller crate → Has been stable, matches D-Bus spec well
