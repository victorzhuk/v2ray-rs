# Design: UI Polish — Status Bar and Logs Improvements

## Context

`status_bar.rs` builds a horizontal `gtk::Box` with "toolbar" CSS class containing a status label and connect button. `logs.rs` uses `gtk::Overlay` where the `adw::StatusPage` ("Process Not Running") overlays the `ScrolledWindow` and toggles visibility via `#[watch] set_visible`.

## Goals / Non-Goals

**Goals:**
- Use `gtk::ActionBar` for status bar (semantic widget for bottom action areas)
- Add icon+label to connect/disconnect button for scannability
- Use `gtk::Stack` with crossfade for logs empty-state transition

**Non-Goals:**
- No log search/filter feature (separate change)
- No status bar layout redesign beyond widget swap

## Decisions

1. **gtk::ActionBar**: Use `pack_start` for status text label, `pack_end` for connect button. ActionBar provides correct spacing, separator line, and toolbar semantics automatically.

2. **Button with icon+label**: Use `set_child` with a horizontal `gtk::Box` containing `gtk::Image` + `gtk::Label`. Icons: `"network-wireless-symbolic"` (connect), `"network-wireless-disabled-symbolic"` (disconnect). Apply `"suggested-action"` when disconnected, `"destructive-action"` when connected.

3. **gtk::Stack for logs**: Replace `gtk::Overlay` with `gtk::Stack` using `StackTransitionType::Crossfade` and 200ms duration. Two named children: `"logs"` (ScrolledWindow) and `"empty"` (StatusPage). Switch via `set_visible_child_name` watched on `model.running`.

## Risks / Trade-offs

- [gtk::Stack vs adw::ViewStack for logs] → gtk::Stack chosen because it supports multiple transition types; adw::ViewStack only supports crossfade and is meant for top-level navigation
