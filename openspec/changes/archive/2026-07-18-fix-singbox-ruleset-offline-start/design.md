# Design: sing-box rule-set bootstrap

## Context

sing-box 1.13.14 downloads each `type: remote` rule-set synchronously inside `StartContext()` when there is no cached copy; any fetch failure aborts startup fatally (`route/rule/rule_set_remote.go:107-112` → `log.Fatal`). The generator currently forces `"download_detour": "direct"`, so the fetch always egresses the real interface — precisely the path that is blocked for `raw.githubusercontent.com` on the networks this app targets. No `experimental.cache_file` is emitted, so even a start that once succeeded (e.g. on a clean network) re-downloads everything next time. Observed live: 14 rule-sets from the RU Bypass preset, every start ends in `FATAL ... initialize rule-set ... TLS handshake timeout`.

## Goals / Non-Goals

- Goal: sing-box starts when GitHub is unreachable directly, provided the proxy node itself is reachable.
- Goal: after one successful fetch, sing-box starts fully offline (cache hit skips the network).
- Non-goal: shipping rule-sets locally / app-managed `.srs` downloads — that is the follow-up change `singbox-local-ruleset-geodata`.
- Non-goal: any change to which rule-sets are referenced (still derived from enabled routing rules).

## Decisions

- Omit `download_detour` instead of pointing it at the proxy tag. sing-box's documented default is the default outbound (`route.final` or first outbound), which is the first proxy outbound in our configs — same effect, no coupling to tag naming, and `download_detour` is deprecated in sing-box 1.14 and removed in 1.16, so not emitting it is the forward-compatible spelling.
- Emit `experimental.cache_file` `{enabled: true, path: "<cache_dir>/sing-box-cache.db"}` whenever at least one remote rule-set is present. Absolute path, because sing-box resolves a bare `cache.db` against its working directory, which we do not control. Placing it in `cache_dir` matches the "cache is regenerable" persistence contract.
- `"store_fakeip": true` is added when `settings.dns.fakeip.enabled` — mappings are in-memory otherwise and go stale across restarts while OS caches still hold the fake IPs.
- The generator gains the cache-dir input via the existing `ConfigWriter` (it already owns `AppPaths`); `ConfigGenerator::generate` keeps its signature by carrying the path in `AppSettings`-adjacent plumbing chosen at implementation time — smallest mechanism wins.

## Risks / Trade-offs

- [First-ever start with an unreachable proxy still fails fatally] → unavoidable with remote rule-sets; resolved by the follow-up local-ruleset change. The failure is now the same failure as "proxy down", which the connect flow already surfaces per candidate.
- [Rule-set downloads through the proxy add first-start latency] → one-time cost per rule-set; cached afterwards.

## Migration Plan

Config-generation-only change; next Connect writes the new shape. Rollback = revert.

## Open Questions

None.
