## 1. Geodata manager

- [ ] 1.1 Add `rule_sets_dir()` (`cache_dir/geodata/rule-sets/`) and per-tag `.srs` path/URL helpers for sing-box
- [ ] 1.2 Implement per-tag fetch for the tags referenced by current routing rules (blocking client behind `geodata-fetch`, atomic writes)
- [ ] 1.3 Remove `.db` download URLs/path helpers; delete stale `geoip.db`/`geosite.db` on refresh
- [ ] 1.4 Wire startup/scheduled/manual refresh to fetch missing `.srs` tags

## 2. Index

- [ ] 2.1 sing-box autocomplete/index source switches off `.db` parsing (static tag list or cached tag enumeration)

## 3. Generator

- [ ] 3.1 `singbox.rs`: emit `type: "local"` with `format: "binary"` and the absolute cached path when the tag's `.srs` exists; `type: "remote"` fallback otherwise
- [ ] 3.2 Tests: local file present → local entry; absent → remote entry; mixed set → mixed entries

## 4. Verification

- [ ] 4.1 `cargo test --workspace` green; `singbox_check` passes with a local-rule-set config against the real binary
- [ ] 4.2 Manual: prime rule-sets via "Update Now", blackhole GitHub, cold-start sing-box connect succeeds
