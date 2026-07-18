# Fix sing-box remote rule-set bootstrap failure

## Why

With any GeoIP/GeoSite routing rule present, the sing-box generator emits `type: remote` rule-sets fetched from `raw.githubusercontent.com` with `"download_detour": "direct"` and no `experimental.cache_file`. sing-box downloads every rule-set synchronously at startup and exits FATAL (`start service: initialize rule-set: ... TLS handshake timeout`) when any fetch fails. On networks where GitHub is blocked or degraded — the primary audience of this app — sing-box therefore cannot start at all, ever: the direct detour guarantees the fetch bypasses the proxy, and the missing cache file guarantees the next start re-downloads from scratch. Verified against sing-box 1.13.14 source: the initial fetch runs only on a cache miss, failure is fatal, and `download_detour` defaults to the default outbound (the proxy) when omitted.

## What Changes

- Drop `"download_detour": "direct"` from generated remote rule-sets. With the field omitted, sing-box downloads through the default outbound — the proxy — which is the only path that works when GitHub is blocked. The proxy outbound dials its own server directly (not through the router), so there is no bootstrap loop.
- Emit `experimental.cache_file` with `"enabled": true` and an explicit absolute `path` under the profile's cache dir whenever the config references at least one remote rule-set, so any successfully fetched rule-set persists and later starts succeed offline. Set `"store_fakeip": true` when FakeIP is enabled, so fakeip↔domain mappings survive restarts.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `config-generator`: "Embed routing rules in config" gains scenarios pinning the sing-box remote rule-set shape: no `download_detour`, and a cache file emitted alongside remote rule-sets.

## Impact

- `crates/core/src/config/singbox.rs` — rule-set emission, new `experimental` section; the generator needs access to the profile cache dir (threaded via `AppSettings` or a generator input alongside it).
- `crates/core/src/config/writer.rs` — pass the cache path into the generator if it is not derivable from `AppSettings`.
- Existing tests asserting `"download_detour": "direct"` flip to asserting its absence.
