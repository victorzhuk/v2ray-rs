## 1. Redaction core

- [ ] 1.1 Implement a `serde_json::Value` walk that masks values at keys `id`, `uuid`, `password`, `short_id` (leaving `public_key`), returning pretty-printed JSON
- [ ] 1.2 Unit tests against generated fixtures for all three backends (assert masked keys per backend, `public_key` visible)

## 2. Dialog

- [ ] 2.1 Add "View Generated Config" to the hamburger `gio::Menu` + `SimpleAction` dispatching a new `AppMsg`
- [ ] 2.2 Build the dialog: monospace read-only TextView in a ScrolledWindow (mirror `logs.rs`), explicit content sizing, header with Refresh, reveal toggle, copy-path button
- [ ] 2.3 Load path via `ConfigWriter::output_path` for the active backend; re-read on open and on Refresh
- [ ] 2.4 Empty state when the file is absent, naming the expected location
- [ ] 2.5 Non-JSON contents: notice + reveal-only viewing (no raw by default)
- [ ] 2.6 Copy-path via `gtk::gdk::Clipboard`

## 3. Verification

- [ ] 3.1 `cargo test --workspace` green; `cargo clippy` clean; manual run: redacted view, reveal toggle, refresh after node toggle, empty state on fresh profile
