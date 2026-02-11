# Design: Main Window UI

## Context

The main window is the primary management interface. It uses Relm4 (Rust GTK4 framework) with libadwaita for modern GNOME-style appearance. The app should work well on GNOME, KDE, and other GTK-supporting Linux desktops.

## Goals / Non-Goals

**Goals:**
- Relm4/GTK4/libadwaita application with modern look
- Tab-based navigation: Subscriptions, Routing, Logs, Settings
- Responsive layout
- Connection status bar always visible
- First-time onboarding wizard
- i18n-ready structure

**Non-Goals:**
- No custom theme engine
- No web-based or Electron UI
- No mobile-responsive design

## Decisions

1. **Framework**: Relm4 0.9+ with GTK4 and libadwaita. Relm4 provides Elm-architecture (Model-Update-View) which maps well to the app's event-driven nature. libadwaita provides AdwApplicationWindow, AdwTabView, and adaptive widgets.

2. **Navigation**: Use `AdwViewStack` with `AdwViewSwitcherBar` for tab navigation. Four pages: Subscriptions, Routing, Logs, Settings.

3. **State management**: Central app model holds references to core services (subscription manager, process controller, etc). Pages communicate via Relm4 message passing. No shared mutable state.

4. **Onboarding**: Use `AdwCarousel` for step-by-step wizard: Welcome → Detect backends → Import first subscription → Done. Only shown on first launch (flag stored in settings).

5. **i18n**: Use `gettext-rs` for translations. Extract strings with `xgettext`. Initial locales: en_US, ru_RU. Store .po/.mo files in `locale/` directory.

## Risks / Trade-offs

- [Relm4 learning curve] Elm architecture in Rust is different from typical GTK patterns → Well-documented, strong community
- [libadwaita dependency] Ties look-and-feel to GNOME → Other DEs still render GTK4 fine, just without adaptive features
