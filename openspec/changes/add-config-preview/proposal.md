## Why

The app's core job is generating backend configs, yet there is no way to see the generated JSON from the UI — debugging a bad config means hunting for a file under the runtime dir. Source: session gap-scan finding "no config preview affordance".

## What Changes

- A "View Generated Config" entry in the main window's hamburger menu opening a dialog with the on-disk generated config for the active backend, in a monospace scrollable view.
- The preview shows the literal file contents re-read from disk (what the backend would actually load), with a Refresh action and a copy-path action; a distinct empty state when the file doesn't exist yet (no enabled nodes, or the tmpfs runtime dir was cleared).
- Credential redaction by default with an explicit reveal toggle (agreed in brainstorm): values at keys `id`, `uuid`, `password`, and Reality `short_id` are masked for display; `public_key` stays visible. Redaction is display-only — the on-disk file is untouched and remains what the backend consumes.
- Copy action copies the file path, not the contents, keeping secrets off the clipboard by default.

## Capabilities

### New Capabilities

- `config-preview`: viewing the generated backend configuration from the UI, including redaction and empty-state behavior.

### Modified Capabilities

_None._ (`main-window` hosts the menu entry point but its requirements don't change; `config-generator` behavior is untouched.)

## Impact

- `crates/ui/src/app.rs` — menu item + `SimpleAction` + new `AppMsg` handler; dialog built from existing patterns (`adw` dialog + ScrolledWindow + monospace TextView per `logs.rs`).
- Redaction as a `serde_json::Value` walk-and-mask over the parsed file before rendering — no generator changes.
- Net-new use of `gtk::gdk::Clipboard` for copy-path (no prior art in the repo, standard GTK4 API).
- Path resolved via `ConfigWriter::output_path` (respects `config_output_dir` override).
