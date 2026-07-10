## Why

Picking DoT/DoQ/H3 as a DNS server protocol on a backend that can't do it (DoT/DoQ/H3 on v2ray, H3 on xray) passes validation cleanly and is silently downgraded to DoH at config-generation time with only an app-log warning — a user who chose DoQ for a reason gets DoH with zero indication. Source: session gap-scan finding "silent DNS protocol downgrade" (`DnsProtocol::fallback_protocol_for_backend`, `crates/core/src/models/dns.rs`).

## What Changes

- The DNS server dialog keeps all protocols selectable but shows an inline warning when the current selection will run as a different protocol on the active backend ("will run as DoH on v2ray") — agreed in brainstorm over hiding options, so a backend switch never silently reinterprets a saved server.
- Saved server rows show a passive downgrade indicator when the active backend will not honor their configured protocol.
- Both UI surfaces derive from the existing core function (`fallback_protocol_for_backend`) — no second compatibility matrix.
- The generator's accept-and-downgrade behavior is unchanged (already spec'd in `config-generator`); `dns-configuration` gains the backend-compatibility matrix as a normative requirement so it stops living only inside generator scenarios.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `dns-preferences-ui`: new requirements — inline downgrade warning in the server dialog and a passive indicator on affected saved rows.
- `dns-configuration`: new requirement — normative per-backend protocol support/downgrade matrix.

## Impact

- `crates/ui/src/preferences/dns.rs` — warning row in the server dialog (existing validation-warning pattern), indicator in `render_dns_servers`; needs the active backend type, which the preferences already receive.
- `crates/core/src/models/dns.rs` — no behavior change; possibly a small helper for "effective protocol differs" reuse.
- Existing generator tests locking in accept-and-downgrade stay untouched.
