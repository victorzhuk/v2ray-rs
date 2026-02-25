# Design: dns-provider-presets

## Context

The routing system has a well-established presets pattern: `Preset` struct with name/description/rules, `builtin_presets()` returning hardcoded presets, `apply_preset()` merging rules, and a UI dialog with per-preset "Apply" buttons. Custom presets can be saved/loaded via persistence.

DNS presets differ from routing presets in shape — they provide server configurations rather than rules. But the UX pattern (built-in list → one-click Apply) is identical.

The `complex-dns-support` change introduces `DnsServerConfig` (tag, protocol, address, port, detour) and `DnsStrategy`. DNS presets will populate these.

## Goals / Non-Goals

**Goals:**
- One-click DNS provider setup for common providers
- Consistent UX with routing presets (dialog with Apply buttons)
- Cover the most popular global and regional DNS providers

**Non-Goals:**
- Custom DNS preset save/load (server configs are simpler than routing rules — not worth the complexity)
- Per-provider DNS rules (presets only set servers + strategy, not DNS routing rules)
- Provider health checking or auto-selection

## Decisions

### D1: Preset struct shape

```rust
pub struct DnsProviderPreset {
    pub name: String,
    pub description: String,
    pub servers: Vec<DnsServerConfig>,
    pub strategy: DnsStrategy,
}
```

Each preset provides a complete server list (typically 2 servers: remote + domestic) and a strategy. Apply replaces `dns.servers` and `dns.strategy` entirely — not a merge.

**Rationale**: DNS server lists are small (2-3 entries). Merging would create duplicates and confusion. Replace semantics are simpler and predictable — "I picked Cloudflare, now I have Cloudflare servers."

### D2: Built-in provider list

| Preset | Remote Server | Domestic Server | Strategy | Description |
|--------|--------------|----------------|----------|-------------|
| Cloudflare | DoH 1.1.1.1 | UDP 1.0.0.1 | PreferIpv4 | Fast, privacy-focused |
| Cloudflare Family | DoH 1.1.1.3 | UDP 1.0.0.3 | PreferIpv4 | Blocks malware + adult content |
| Google | DoH 8.8.8.8 | UDP 8.8.4.4 | PreferIpv4 | Reliable, global |
| AdGuard | DoH dns.adguard.com | UDP 94.140.14.14 | PreferIpv4 | Blocks ads + trackers |
| AdGuard Family | DoH dns-family.adguard.com | UDP 94.140.14.15 | PreferIpv4 | Blocks ads + adult content |
| Quad9 | DoH dns.quad9.net | UDP 9.9.9.9 | PreferIpv4 | Blocks malware, privacy-focused |
| Ali DNS | DoH dns.alidns.com | UDP 223.5.5.5 | PreferIpv4 | Optimized for China |
| Yandex DNS | DoH common.dot.dns.yandex.net | UDP 77.88.8.8 | PreferIpv4 | Optimized for Russia |

Remote server uses DoH (encrypted, routed through proxy), domestic uses UDP (fast, direct).

The remote server gets tag "remote" and the domestic gets tag "domestic" — matching the default convention from `complex-dns-support`.

### D3: Apply behavior

Applying a preset:
1. Replaces `dns.servers` with the preset's servers
2. Sets `dns.strategy` to the preset's strategy
3. Enables DNS if not already enabled (`dns.enabled = true`)
4. Does NOT touch: `dns.rules`, `dns.use_custom_rules`, `dns.fakeip`, `dns.disable_cache`, `dns.client_subnet`, `dns.hosts`

**Rationale**: Presets answer "which DNS servers to use" — they don't interfere with rules, caching, or other advanced settings the user may have configured.

### D4: UI placement

Add a "Providers" button (with `starred-symbolic` icon) in the DNS page Servers group header. Clicking opens an `adw::AlertDialog` listing providers as `adw::ActionRow`s with name, description, and "Apply" button — same pattern as `show_routing_presets_dialog()`. Since applying a preset replaces the server list (unlike routing presets which merge), the Apply action SHALL show a brief confirmation ("Replace current DNS servers with {provider}?") before proceeding.

**Rationale**: Reuses the proven routing presets dialog UX. Placing the button in the Servers group header makes it discoverable right where servers are managed.

## Risks / Trade-offs

- **[Stale provider info]** Provider addresses/features could change → Low risk for well-known providers; addresses are stable. Can update in future releases.
- **[Replace semantics lose custom servers]** Apply wipes user's custom servers → Mitigated by confirmation dialog before replace. Users can re-add custom servers or modify the preset's servers after applying.
