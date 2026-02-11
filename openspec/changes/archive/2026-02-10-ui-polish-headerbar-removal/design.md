# Design: UI Polish — HeaderBar Removal & Navigation Chrome

## Context

The main window uses `adw::ViewStack` + `adw::ViewSwitcher` for tab navigation. Three of four sub-pages (subscriptions, routing, logs) each render their own `adw::HeaderBar` with a title label and action buttons. The fourth page (settings) correctly omits a HeaderBar — it's just an `adw::PreferencesPage` trusting the parent window chrome. This inconsistency creates a double-title-bar on 3 of 4 pages.

## Goals / Non-Goals

**Goals:**
- Remove nested HeaderBars from subscriptions, routing, logs pages
- Relocate page action buttons into the page content area
- Add symbolic icons to ViewStack tabs
- Remove unnecessary widget wrappers in app.rs

**Non-Goals:**
- No widget type changes (ListBox migration, ExpanderRow, ActionBar — separate changes)
- No i18n wiring (separate change)
- No functionality changes — all buttons and actions preserved, just relocated

## Decisions

1. **Action button placement after HeaderBar removal**: Place page action buttons in a horizontal `gtk::Box` at the top of the page content, right-aligned with `gtk::Align::End`. This matches GNOME Files, GNOME Settings, and similar apps that put contextual actions in the content area. Alternative considered: floating action button (FAB) via `gtk::Overlay` — rejected because FABs are uncommon in GNOME desktop apps.

2. **ViewStack icon names**: Use standard Adwaita symbolic icons:
   - Subscriptions: `folder-download-symbolic`
   - Routing: `network-server-symbolic`
   - Logs: `utilities-terminal-symbolic`
   - Settings: `preferences-system-symbolic`

3. **Action button styling**: Use `"flat"` CSS class for relocated action buttons to match GNOME content-area button patterns. Keep existing `"suggested-action"` or `"destructive-action"` classes where already used.

## Risks / Trade-offs

- [Toolbar buttons less discoverable in content area] → Use tooltips on all icon buttons; group them visually at top-right of each page
- [Relm4 view! macro differences] → The pages use imperative widget building in `init()`, not declarative view! macro, so restructuring is straightforward
