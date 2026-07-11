## Context

Generated configs land at `ConfigWriter::output_path` — `{runtime_dir}/generated/{v2ray,xray,sing-box}.json` (0700 dir, atomic writes), or `config_output_dir` when overridden. Files are written proactively on node/settings changes while disconnected and per-candidate at connect; absent only before any node is enabled or after tmpfs cleanup. Credential keys are backend-specific: v2ray/xray use `id` + `password`; sing-box uses `uuid` + `password`; Reality adds `shortId` (v2ray/xray) or `short_id` (sing-box) as a borderline secret, while `public_key` is not secret. Menu prior art: single-item `gio::Menu` + `SimpleAction`; monospace view prior art: `logs.rs`; dialog prior art: `adw::AlertDialog` with `set_extra_child`.

## Goals / Non-Goals

- Goal: show exactly what the backend would load; make credentials shoulder-surf-safe by default.
- Non-goal: pretending the on-disk file is protected — it stays cleartext (that's what the backend reads); redaction is a display concern, framed as such.
- Non-goal: config editing, diffing, or live regeneration from the dialog.

## Decisions

- Re-read from disk rather than regenerate: a synthesized view can diverge from what the running backend loaded — useless for debugging. Refresh = re-read.
- Redaction as post-hoc `serde_json::Value` walk masking values at `{id, uuid, password, short_id, shortId}` (union across backends — v2ray/xray emit `shortId`, sing-box emits `short_id`; masking per-backend key subsets risks leaking `uuid` while on xray, etc.). Alternative — threading a redact flag through three generators — rejected: display concern doesn't belong in generation.
- Non-JSON file (corrupt/partial): stay redacted-by-default — show notice, reveal only via the toggle. Never silently fall back to raw.
- Copy-path over copy-contents: keeps secrets off the clipboard; the path is what's needed for `sing-box check`-style debugging anyway.
- Dialog: `adw::Dialog`/`AlertDialog` + ScrolledWindow + monospace TextView with explicit content sizing (AlertDialog is form-sized by default; a JSON blob needs width/height set).

## Risks / Trade-offs

- [Wrong key set leaks a credential] → mask the union of all backends' keys; test per backend fixture.
- [New `gdk::Clipboard` surface] → standard GTK4 API, one call site.
- [Stale view mislead] → Refresh button + file mtime displayed if cheap.

## Migration Plan

UI-only PR. Rollback = revert.

## Open Questions

None.
