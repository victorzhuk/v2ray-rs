# Design: UI Polish — Lists, Expanders, and Button Consolidation

## Context

The subscription and routing pages build list UIs imperatively in `init()` and `render_list()` helper functions. Both use `gtk::Box` as list containers with manual child management. Subscriptions track expand/collapse state via `expanded_subs: HashSet<Uuid>` and re-render the entire list on toggle. The wizard creates independent CheckButtons in a loop without radio grouping.

## Goals / Non-Goals

**Goals:**
- Replace `gtk::Box` list containers with `gtk::ListBox` + `"boxed-list"` for Adwaita-styled lists
- Replace manual expand/collapse with `adw::ExpanderRow` in subscriptions
- Consolidate excess suffix buttons into menu popovers
- Fix wizard radio button grouping
- Fix debug formatting in user-facing text
- Improve protocol badge visibility

**Non-Goals:**
- No drag-and-drop changes in routing (keep existing DragSource/DropTarget)
- No functional changes to subscription/routing CRUD operations
- No i18n wiring (separate change)

## Decisions

1. **ExpanderRow for subscriptions**: Use `adw::ExpanderRow` with `show_enable_switch(true)` to combine the expand toggle and enabled switch into one widget. This eliminates the `expanded_subs` HashSet, `ToggleExpand` message, and manual icon switching. Child node rows are added via `expander.add_row()`.

2. **Menu popover approach**: Use `gtk::Popover` with a vertical `gtk::Box` containing frameless buttons, attached to a `gtk::MenuButton` with `"view-more-symbolic"` icon. Chosen over `gio::Menu` model approach because direct button connections to Relm4 senders are simpler than setting up action groups.

3. **ListBox migration**: Change `list_container` field type from `gtk::Box` to `gtk::ListBox`. The `render_list()` functions already use `.first_child()` / `.remove()` which work identically on both types. Add `"boxed-list"` CSS class for rounded borders and proper Adwaita styling.

4. **Radio button grouping**: Collect the first `CheckButton` created in the loop, then call `.set_group(Some(&first))` on subsequent buttons. This must happen in `init()` since `set_group()` cannot be called in the Relm4 view! macro.

5. **Debug formatting fix**: Add `format_action()` and `format_match()` helper functions returning human-readable `&str` instead of relying on `{:?}` Debug formatting.

## Risks / Trade-offs

- [ExpanderRow enable_switch semantics differ from current Switch] → Current switch toggles subscription enabled state; ExpanderRow's enable_switch controls expansion. Need to map `enable_expansion_notify` signal to the existing `ToggleSubscription` message
- [Menu popover adds click depth for common actions] → Keep the most-used action (enable/disable switch) visible; only move secondary actions (update, delete, move) into the menu
