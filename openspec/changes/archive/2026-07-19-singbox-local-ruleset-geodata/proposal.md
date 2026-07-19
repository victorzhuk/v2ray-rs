# Replace dead sing-box .db geodata with app-managed .srs rule-sets

## Why

`GeodataManager` still downloads `geoip.db`/`geosite.db` for sing-box (SagerNet release assets), but sing-box removed `.db` support in 1.12.0 — the packaged binary is 1.13.14. The files sit unused in the cache (both present on disk today, dated from an old install) while the generator references remote rule-sets instead: the "Update geodata" surface silently maintains data the backend cannot read, and sing-box's actual geodata (per-tag `.srs` files) is not app-managed at all. Consequence: sing-box's first start on a GitHub-blocked network has nothing local to fall back to, and the geodata autocomplete index for sing-box is built from dead files.

## What Changes

- `GeodataManager` for sing-box downloads per-tag binary rule-set files (`geoip-<tag>.srs`, `geosite-<tag>.srs`) from the `rule-set` branches of `SagerNet/sing-geoip` / `SagerNet/sing-geosite` into `cache_dir/geodata/rule-sets/`, for the tags referenced by the current routing rules; stops downloading `geoip.db`/`geosite.db` and deletes stale copies.
- The sing-box generator emits `type: "local"` rule-sets pointing at cached `.srs` files when present, falling back to `type: "remote"` (per `fix-singbox-ruleset-offline-start`) for tags not yet cached.
- Routing-rule changes that introduce a new GeoIP/GeoSite tag trigger a background fetch of the missing `.srs` on the existing geodata refresh paths (startup refresh, scheduled refresh, manual "Update Now").

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `geodata-management`: "Backend-specific geodata format" — sing-box uses per-tag `.srs` rule-sets, not `.db`; download and refresh requirements follow.
- `config-generator`: sing-box rule-set emission prefers local cached files.

## Impact

- `crates/core/src/geodata.rs` — sing-box download URLs, per-tag fetch, stale `.db` cleanup.
- `crates/core/src/geodata_index.rs` — sing-box index source becomes the cached rule-set tags (or the static tag list), not `.db` parsing.
- `crates/core/src/config/singbox.rs` — local-vs-remote rule-set decision per tag.
- `crates/ui` geodata preferences — no UI change beyond honest counts.
