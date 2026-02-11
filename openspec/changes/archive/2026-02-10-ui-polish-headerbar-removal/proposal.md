# Proposal: UI Polish — HeaderBar Removal & Navigation Chrome

## Why

The UI has a double-title-bar anti-pattern: every sub-page (subscriptions, routing, logs) renders its own `adw::HeaderBar` below the main ViewSwitcher header, wasting ~40px vertical space and violating GNOME HIG. The ViewStack tabs also lack icons, making navigation less scannable. These are the highest-impact visual issues identified in the UI review.

## What Changes

- Remove `adw::HeaderBar` from subscriptions, routing, and logs pages (settings.rs already does it right — no HeaderBar)
- Move page-specific action buttons (Add Subscription, Add Rule, Presets, Clear Logs) into the page content area
- Add symbolic icons to all ViewStack tab pages via `add_titled_with_icon()`
- Remove unnecessary `gtk::Box` wrapper around wizard widget in app.rs

## Capabilities

### New Capabilities
(none — this is a polish/fix change, not new functionality)

### Modified Capabilities
(none — no spec-level behavior changes, only layout/presentation)

## Impact

- Modified files: `crates/ui/src/subscriptions.rs`, `crates/ui/src/routing.rs`, `crates/ui/src/logs.rs`, `crates/ui/src/app.rs`
- No API changes, no dependency changes, no data model changes
- Pure presentation layer refactor — all existing functionality preserved
