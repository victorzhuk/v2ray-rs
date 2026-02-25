# Proposal: complex-dns-support

## Why

The app already has a minimal `DnsConfig` model (remote/domestic server pair with Plain/DoH) and backend config generation, but zero UI to configure it — users are stuck with hardcoded defaults. Real-world geo-routing setups require fine-grained DNS control: multiple named servers with different protocols, DNS-specific routing rules, IP strategy selection, FakeIP for sing-box, cache tuning, and EDNS client subnet. Without this, users must hand-edit generated configs or accept suboptimal DNS resolution.

## What Changes

- **Expand DNS domain model**: Replace the current `DnsConfig` (2 fixed servers, 1 protocol enum) with a rich model supporting multiple named DNS servers, 6 protocol types (UDP, TCP, DoH, DoT, DoQ, H3), DNS-specific routing rules, IP query strategy, FakeIP configuration, cache controls, EDNS client subnet, and static host overrides
- **Update config generation**: Rewrite `build_dns()` in both v2ray and sing-box generators to emit the full DNS configuration from the new model, including per-server settings, DNS rules, hosts, queryStrategy/strategy, FakeIP (sing-box), and cache options
- **Add DNS Preferences page**: New page in the Preferences dialog with controls for all DNS settings — server list management, protocol selection, DNS rules, strategy picker, FakeIP toggle (sing-box-conditional), cache settings, client subnet, and hosts table
- **Persist DNS config**: The expanded `DnsConfig` continues to live in `AppSettings` (TOML), with backward-compatible `#[serde(default)]` deserialization

## Capabilities

### New Capabilities
- `dns-configuration`: DNS domain model with multiple named servers (6 protocols), DNS routing rules, IP strategy, FakeIP, cache controls, EDNS client subnet, and static host overrides
- `dns-preferences-ui`: Preferences page for configuring all DNS settings with backend-aware conditional display (e.g., FakeIP only shown for sing-box)

### Modified Capabilities
- `config-generator`: Rewrite `build_dns()` for both backends to emit full DNS config from expanded model — multiple servers with tags/detours, DNS rules, hosts, queryStrategy, FakeIP, cache, client subnet

## Impact

- **Models**: `crates/core/src/models/dns.rs` — full rewrite of `DnsConfig`, `DnsServer`, `DnsProtocol`; new types: `DnsRule`, `DnsStrategy`, `FakeIpConfig`, `HostOverride`
- **Config gen**: `crates/core/src/config/v2ray.rs` and `singbox.rs` — rewrite `build_dns()` functions
- **UI**: `crates/ui/src/preferences.rs` — new DNS page added to Preferences dialog
- **Persistence**: No schema change needed — `DnsConfig` stays in `AppSettings`, serde handles backward compat via defaults
- **Dependencies**: No new crate dependencies expected
- **Depends on**: config-generator, app-persistence, main-window (Preferences dialog host)
