# Tasks: UI Polish — Status Bar and Logs Improvements

## 1. Status Bar

- [x] 1.1 Replace root `gtk::Box` with `gtk::ActionBar` in `status_bar.rs` — use `pack_start` for status label, `pack_end` for connect button
- [x] 1.2 Replace connect button text-only with icon+label child: horizontal `gtk::Box` containing `gtk::Image` + `gtk::Label`, watched on `model.connected`
- [x] 1.3 Apply `"suggested-action"` CSS when disconnected, `"destructive-action"` when connected via `#[watch] set_css_classes`

## 2. Logs Page

- [x] 2.1 Replace `gtk::Overlay` with `gtk::Stack` in `logs.rs` — set `transition_type: Crossfade`, `transition_duration: 200`
- [x] 2.2 Add `adw::StatusPage` as named child `"empty"` and `gtk::ScrolledWindow` as named child `"logs"` in the Stack
- [x] 2.3 Switch visible child via `#[watch] set_visible_child_name` based on `model.running`
