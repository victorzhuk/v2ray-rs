## Why

A `settings.toml` whose `[dns]` table omits `enabled` fails to deserialize — `DnsConfigWire.enabled` carries no serde default (`crates/core/src/models/dns.rs`), `load_settings` maps any deserialize error to `CorruptConfig`, and `load_settings_or_default` then returns `AppSettings::default()` (`crates/core/src/persistence/settings.rs`), so one missing key silently discards every other setting in the file. A cleared server list does not survive a reload either: `From<DnsConfigWire>` substitutes the two default servers whenever `servers` deserializes to an empty vector, so it cannot tell "the user removed every server" from "the key is absent". A server downgraded to DoH keeps the port configured for its original protocol, because `dns_server_address_for_backend` passes `server.port` to the address formatter regardless of the protocol it had to fall back to (`crates/core/src/config/v2ray.rs`), producing `https://dns.google:853/dns-query`.

## What Changes

- A `[dns]` table without `enabled` loads with `enabled = false` instead of making the whole settings file unreadable.
- An explicitly present `servers` key is loaded as written, including an empty list; the two default servers are supplied only when the key is absent and no legacy `remote`/`domestic` fields are present.
- A server whose protocol is downgraded for the selected backend is emitted with the DoH default port rather than the port set for its original protocol.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `dns-configuration`: new requirements for lenient deserialization of partial `[dns]` tables and port handling on downgrade.

## Impact

- `crates/core/src/models/dns.rs` — `DnsConfigWire` defaults and `From<DnsConfigWire>` migration.
- `crates/core/src/config/v2ray.rs` — downgraded address formatting in `dns_server_address_for_backend`.
- `crates/core/src/persistence/settings.rs` — covered by a load test; no code change expected.
- Settings file format unchanged; previously unloadable files now load.