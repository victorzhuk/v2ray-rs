# Proposal: dns-provider-presets

## Why

Configuring DNS servers manually (protocol, address, port) requires users to know provider-specific details. Most users just want "use Cloudflare" or "use AdGuard with ad blocking". The app already has a routing presets pattern (built-in + custom presets with one-click Apply) that works well — DNS should have the same convenience. This depends on the `complex-dns-support` change being implemented first.

## What Changes

- **Built-in DNS provider presets**: Hardcoded list of well-known DNS providers, each providing a pair of `DnsServerConfig` entries (remote + domestic) and a suggested strategy. Providers include: Cloudflare, Cloudflare Family, Google, AdGuard, AdGuard Family, Quad9, Ali DNS, Yandex DNS
- **Apply preset replaces servers**: One-click "Apply" replaces the current DNS server list with the provider's servers and sets the strategy
- **Provider picker UI**: A "Providers" button in the DNS Preferences Servers group opens a dialog listing available providers with descriptions and Apply buttons (same UX pattern as routing presets)

## Capabilities

### New Capabilities
- `dns-provider-presets`: Built-in DNS provider presets with one-click Apply, following the routing presets pattern

### Modified Capabilities
- `dns-preferences-ui`: Add "Providers" button to the Servers group that opens the provider picker dialog

## Impact

- **Models**: `crates/core/src/models/dns.rs` — new `DnsProviderPreset` struct and `builtin_dns_presets()` function
- **UI**: `crates/ui/src/preferences.rs` — add Providers button + dialog in DNS page Servers group
- **No persistence changes**: Built-in only, no custom DNS preset save/load
- **Depends on**: `complex-dns-support` (requires the expanded DNS model with `DnsServerConfig`, `DnsStrategy`)
