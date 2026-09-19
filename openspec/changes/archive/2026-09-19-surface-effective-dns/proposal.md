## Why

The resolvers a connection actually uses are often not the ones shown in DNS preferences, and nothing tells the user. With DNS disabled, xray TUN resolves through hardcoded `https://1.1.1.1/dns-query` and `https://8.8.8.8/dns-query` via the proxy and ignores the configured servers; sing-box TUN derives a DoH `1.1.1.1` resolver; xray appends a fallback when every server is domain-scoped; bootstrap resolvers go out directly for node hostnames; outside TUN with DNS disabled the OS resolver answers routing's geoip lookups; a detour-less sing-box server dials directly; and a subscription's imported profile replaces DNS and routing altogether. When DNS misbehaves, the user cannot tell which resolver answered or which path it took.

## What Changes

- DNS preferences gain a read-only "Effective DNS" summary for the selected backend and the current TUN state: each resolver with its transport, path (direct, proxy, backend routing, or system resolver), source (user, fallback, bootstrap, system), and scope (all domains or listed domains). It names subscriptions whose enabled imported profile replaces DNS for their nodes.
- Every backend launch writes the same summary, computed from the settings actually used for that candidate (imported profile and connect-time host pins included), to `backend.log` right after the session record.
- When a connection runs on fallback resolvers (xray or sing-box TUN with DNS disabled, or xray with only domain-scoped servers), one notice line per connection in the process log stream says so and names the setting that changes it.
- No change to which resolvers are generated.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `dns-preferences-ui`: new read-only effective DNS summary.
- `dns-configuration`: new requirement defining the effective resolver set and how each entry is classified.
- `diagnostic-logs`: effective DNS record per launch and the fallback notice.

## Impact

- `crates/core/src/config/` — new pure effective-DNS module next to the generators; no generator change.
- `crates/ui/src/preferences/dns.rs` — summary group, refreshed on settings change.
- `crates/ui/src/connection.rs` — computes the summary per candidate, writes the `dns` records, emits the notice.
- `crates/process/src/manager.rs` — writes caller-supplied records after the session record.
