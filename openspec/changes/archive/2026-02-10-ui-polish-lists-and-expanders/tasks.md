# Tasks: UI Polish — Lists, Expanders, and Button Consolidation

## 1. ListBox Migration

- [x] 1.1 Change `list_container` field from `gtk::Box` to `gtk::ListBox` in `subscriptions.rs`, add `"boxed-list"` CSS class, set `SelectionMode::None`
- [x] 1.2 Change `list_container` field from `gtk::Box` to `gtk::ListBox` in `routing.rs`, add `"boxed-list"` CSS class, set `SelectionMode::None`

## 2. ExpanderRow in Subscriptions

- [x] 2.1 Replace `adw::ActionRow` + manual expand button with `adw::ExpanderRow` in `build_subscription_group()`
- [x] 2.2 Move node rows from conditional `group.add()` to `expander.add_row()` (always added, visibility controlled by widget)
- [x] 2.3 Remove `expanded_subs: HashSet<Uuid>` field from `SubscriptionsPage` struct
- [x] 2.4 Remove `ToggleExpand(Uuid)` variant from `SubscriptionsMsg` and its match arm in `update()`
- [x] 2.5 Remove `expanded` parameter from `render_list()` and `build_subscription_group()` signatures

## 3. Button Consolidation

- [x] 3.1 In `subscriptions.rs`: replace inline update/delete suffix buttons with a `gtk::MenuButton` + `gtk::Popover` containing "Update" and "Delete" frameless buttons
- [x] 3.2 In `routing.rs`: replace inline up/down/edit/delete suffix buttons with a `gtk::MenuButton` + `gtk::Popover` containing "Move Up", "Move Down", "Edit", "Delete" frameless buttons (conditionally include Move Up/Down based on position)

## 4. Wizard Fixes

- [x] 4.1 Link backend `CheckButton` instances with `set_group()` in `init()` — collect first button, call `set_group(Some(&first))` on subsequent
- [x] 4.2 Replace "No Backend Found" `PreferencesGroup` + `Label` with `adw::StatusPage` using `"dialog-error-symbolic"` icon

## 5. Visual Polish

- [x] 5.1 Add `format_action()` helper in `routing.rs` returning `&'static str` for each `RuleAction` variant, replace all `format!("{:?}", rule.action)` usages
- [x] 5.2 Change protocol badge CSS from `["dim-label"]` to accent styling in `build_node_row()`
