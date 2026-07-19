# Design: sing-box local rule-sets

## Context

sing-box 1.8 introduced rule-sets and deprecated `.db` geodata; 1.12 removed `.db` entirely. A local rule-set is `{"type": "local", "tag": ..., "format": "binary", "path": ".../geosite-<tag>.srs"}`; the per-tag `.srs` files live on the `rule-set` branches of SagerNet/sing-geoip and SagerNet/sing-geosite — the same URLs the generator already uses for `type: remote`. Local rule-set paths auto-reload on mtime change since sing-box 1.10.

## Goals / Non-Goals

- Goal: sing-box first start succeeds with GitHub blocked, using app-downloaded `.srs` files.
- Goal: "Update geodata" maintains data the packaged sing-box can actually read.
- Non-goal: bundling `.srs` files in the package (licensing/size; download stays runtime).
- Non-goal: changing xray/v2ray `.dat` handling.

## Decisions

- Download set = tags referenced by current routing rules (bounded, typically < 20 files of a few hundred KB) rather than mirroring the full upstream catalogs (hundreds of files). The catalog is only needed for autocomplete, which keeps using the static tag list already shipped for validation.
- Local file wins per tag: generator checks `cache_dir/geodata/rule-sets/<tag>.srs` existence at generation time; missing tags fall back to `type: remote` so a half-fetched cache still yields a startable config (remote + cache_file from the prior change).
- Fetches reuse the existing `geodata-fetch` blocking-reqwest path and the async wrapper pattern already used for `.dat` downloads (`spawn_local` + `spawn_blocking`); failures leave the previous file in place (atomic write via tempfile, same as other persistence).
- `.db` handling is deleted, including download URLs, path helpers, and the sing-box branch of the reindex; stale `geoip.db`/`geosite.db` in the cache are removed on the first refresh. Cache dir is documented as regenerable, so deletion is safe.

## Risks / Trade-offs

- [Rule edits reference a tag before its `.srs` arrives] → config falls back to remote for that tag; connectivity identical to today's behavior post-`fix-singbox-ruleset-offline-start`.
- [Upstream branch layout changes] → same exposure the remote rule-set URLs already have; single constant to update.

## Migration Plan

First refresh after upgrade deletes `.db` files and fetches `.srs` for referenced tags. No settings migration. Rollback = revert; remote rule-sets keep working.

## Open Questions

None.
