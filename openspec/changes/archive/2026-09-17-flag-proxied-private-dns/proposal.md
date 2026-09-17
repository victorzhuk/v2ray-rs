## Why

A DNS server can be addressed at the proxy server's own network and the page says nothing. The live settings carry `domestic` = `udp 127.0.0.1` with detour `proxy`, and the generated xray config sends that server's queries through the proxy to the remote host's loopback, so domestic lookups fail with no hint in the UI (`crates/core/src/config/v2ray.rs` tags the server `dns-internal`, and routing sends that tag to the proxy). The detour control reads as a preference, not as a statement about which network the resolver is reached from: sing-box emits `detour: <first proxy>` for any detour other than `direct` and omits it otherwise, and xray tags a server direct only under TUN. Separately, the IP strategy selector offers "Prefer IPv4" and "Prefer IPv6" on xray and v2ray, where `query_strategy_str` maps both Prefer and the Only variants onto `UseIPv4`/`UseIPv6` — family-only on those backends, which the page does not say.

## What Changes

- A DNS server addressed by a loopback, private, link-local, or unique-local IP literal whose queries the generated config sends through the proxy is flagged: an inline warning in the server dialog, the same warning on its row and in the primary section, and one warning log line per flagged server at config generation. Saving and connecting stay allowed.
- The IP strategy selector notes, for xray and v2ray, that the Prefer options query only the preferred address family on that backend. Generated values are unchanged.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `dns-configuration`: new requirement for private DNS servers routed through the proxy.
- `dns-preferences-ui`: new requirements for the private-server warning and the per-backend strategy note.

## Impact

- `crates/core/src/models/dns.rs` — private-server predicate on `DnsServerConfig`.
- `crates/core/src/config/v2ray.rs`, `crates/core/src/config/singbox.rs` — one warning per flagged server during generation.
- `crates/ui/src/preferences/dns.rs` — dialog warning, server-row and primary-row warning, strategy-row note.