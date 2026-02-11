# Proposal: UI Polish — Lists, Expanders, and Button Consolidation

## Why

Subscription and routing list items use raw `gtk::Box` containers instead of semantic `gtk::ListBox` with Adwaita styling. Subscriptions use manual expand/collapse state tracking (~30 lines + `expanded_subs: HashSet<Uuid>`) instead of `adw::ExpanderRow`. Every list row has 4-5 suffix buttons (toggle, expand, update, delete / switch, up, down, edit, delete) which overwhelms the UI. The wizard's backend selection uses unlinked CheckButtons that visually allow multiple selections.

## What Changes

- Migrate subscription and routing list containers from `gtk::Box` to `gtk::ListBox` with `"boxed-list"` CSS class
- Replace manual expand/collapse in subscriptions with `adw::ExpanderRow` (removes `expanded_subs` state field and `ToggleExpand` message)
- Consolidate 4-5 suffix buttons per row to max 2 visible + `gtk::MenuButton` popover for secondary actions
- Fix wizard `CheckButton` instances to use `set_group()` for radio button behavior
- Replace `{:?}` debug formatting for `RuleAction` with human-readable strings
- Improve protocol badge styling from `"dim-label"` to accent styling
- Use `adw::StatusPage` for "No Backend Found" state in wizard

## Capabilities

### New Capabilities
(none)

### Modified Capabilities
(none — presentation-only changes, no spec-level behavior changes)

## Impact

- Modified files: `crates/ui/src/subscriptions.rs`, `crates/ui/src/routing.rs`, `crates/ui/src/wizard.rs`
- State simplification in subscriptions: removes `expanded_subs: HashSet<Uuid>` field, `ToggleExpand` message variant, and related match arm
- No API changes, no dependency changes
