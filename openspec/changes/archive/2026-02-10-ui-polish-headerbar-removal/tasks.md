# Tasks: UI Polish — HeaderBar Removal & Navigation Chrome

## 1. Remove Nested HeaderBars

- [x] 1.1 Remove `adw::HeaderBar` from `subscriptions.rs` — relocate "Add Subscription" button to a right-aligned `gtk::Box` at top of page content with `"flat"` CSS class
- [x] 1.2 Remove `adw::HeaderBar` from `routing.rs` — relocate "Add Rule" and "Presets" buttons to a right-aligned `gtk::Box` at top of page content with `"flat"` CSS class
- [x] 1.3 Remove `adw::HeaderBar` from `logs.rs` — relocate "Clear Logs" button to a right-aligned `gtk::Box` at top of page content with `"flat"` CSS class

## 2. ViewStack Icons

- [x] 2.1 Replace `add_titled` with `add_titled_with_icon` in `app.rs` for all four ViewStack pages: Subscriptions (`folder-download-symbolic`), Routing (`network-server-symbolic`), Logs (`utilities-terminal-symbolic`), Settings (`preferences-system-symbolic`)

## 3. Widget Cleanup

- [x] 3.1 Remove unnecessary `gtk::Box` wrapper around wizard widget in `app.rs` — render wizard widget directly
