# Proposal: UI Polish — Status Bar and Logs Improvements

## Why

The status bar uses a raw `gtk::Box` with manual "toolbar" CSS class instead of the semantic `gtk::ActionBar` widget. The connect/disconnect button has no icon, making it harder to scan. The logs page uses `gtk::Overlay` with abrupt `set_visible` toggling between the empty state and log content — a `gtk::Stack` with crossfade transition would be smoother and semantically correct (it's mutual-exclusion, not overlay).

## What Changes

- Replace status bar `gtk::Box` with `gtk::ActionBar` for proper bottom-bar semantics
- Add icon to the connect/disconnect button (icon + label)
- Replace logs `gtk::Overlay` visibility toggle with `gtk::Stack` + crossfade transition
- Apply `"destructive-action"` / `"suggested-action"` CSS classes to connect button based on state

## Capabilities

### New Capabilities
(none)

### Modified Capabilities
(none — presentation-only changes)

## Impact

- Modified files: `crates/ui/src/status_bar.rs`, `crates/ui/src/logs.rs`
- No API changes, no dependency changes
