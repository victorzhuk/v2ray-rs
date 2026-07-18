# Design: self-contained DNS for TUN mode

## Context

Verified against Xray docs and the live failing setup (xray 26.3.27, TUN on, DNS settings off, RU Bypass rules):

- Xray's built-in DNS traffic can be tagged (`dns.tag`) and matched by `inboundTag` routing rules; with no matching rule it falls to the first outbound. No per-server sockopt exists — the mark comes from whichever outbound the query is routed through.
- Sniffing `destOverride` (with default `routeOnly: false`) discards the original destination IP; `freedom` with `AsIs` then hands the hostname to the OS resolver at dial time. `UseIP` forces the built-in resolver instead.
- `IPIfNonMatch` resolves sniffed domains through the built-in resolver for `geoip` rule matching; with no `dns` section that resolver is `localhost` — the OS resolver.
- Poisoned OS-resolver answers for blocked domains land in RU address space → `geoip:ru → direct` → DPI resets the direct dial. This is the mechanism behind the observed `proxy/tun: connection reset by peer` / `connection was refused` errors and Claude Code stream drops.
- xray's `tun` inbound first shipped in v26.1.13; `autoSystemRoutingTable` self-routing exists only in v26.6.27+ (netctl remains the route path for all supported versions).
- sing-box: the `hijack-dns` rule and `dns` block are both gated on `dns.enabled` today, so `dns_hijack` is dead weight while DNS settings are off.

## Goals / Non-Goals

- Goal: no resolution that feeds routing decisions or direct dials ever uses the OS resolver while TUN is on.
- Goal: application DNS captured by the TUN is answered by the backend's resolver when `dns_hijack = Hijack`.
- Goal: TUN with an incapable xray fails with an actionable message.
- Non-goal: changing non-TUN (SOCKS/HTTP-only) DNS behavior — apps talking to the local proxy ports already send domains to the proxy.
- Non-goal: DoH bootstrap sophistication — `1.1.1.1` is an IP-literal DoH endpoint precisely so no bootstrap resolution is needed.
- Non-goal: raising `connIdle` (600s today); revisit only if stream drops persist after DNS is fixed.

## Decisions

- Derived-DNS shape (both backends, only when `tun.enabled && !dns.enabled`): single DoH server `https://1.1.1.1/dns-query` routed through the first proxy outbound. One server keeps the derivation trivially correct; users needing split-horizon or domestic resolution enable the DNS feature, which already models servers/rules/detours. The derivation is generation-time only — settings are not mutated.
- All xray internal-resolver queries route through the proxy via one `inboundTag` rule rather than per-server plumbing: xray cannot mark DNS sockets, and any "+local" server form bypasses routing into unmarked direct sockets, which under the TUN policy rules would loop or leak. When user DNS is enabled with a "domestic" plain-UDP server, that server's queries also traverse the proxy; the poisoning-resistance win outweighs geo-CDN affinity, and the trade-off is documented in the proposal.
- The hijack rule (`udp/53 → dns-out`) is ordered after the `inboundTag` rule and before everything else, so hijacked app queries cannot themselves be captured into rule evaluation loops; `exclude_domains` DNS still resolves direct per the existing exclusion requirement.
- xray version gate lives in the connect preflight next to the existing `CAP_NET_ADMIN` gate (process crate already receives the backend version string from detection); config-generation stays version-agnostic. Threshold constant `26.1.13`.
- sing-box changes are confined to `build_dns` gating: `settings.dns.enabled || settings.tun.enabled` drives emission; the derived path synthesizes the single-DoH server list with `detour` = first proxy tag. The existing `hijack-dns`/`default_domain_resolver` logic then works unchanged.

## Risks / Trade-offs

- [DoH endpoint 1.1.1.1 unreachable via the proxy] → resolution fails visibly rather than silently poisoned; same blast radius as "proxy down". Mitigation: users can enable DNS settings and pick any preset.
- [`UseIP` on freedom changes direct-dial semantics for genuinely-RU domains] → they now dial the built-in resolver's answer (fetched via proxy) instead of the OS answer; for unpoisoned domains these agree modulo CDN geo-affinity.
- [Behavior change for existing TUN users with DNS off] → this is the bug being fixed; the previous behavior was silently poisonable.
- [Hijack intercepts all UDP:53 including LAN-resolver traffic] → intended semantics of `Hijack`; `Native` remains available.

## Migration Plan

Config-generation plus one preflight check; next Connect applies it. Rollback = revert.

## Open Questions

- Whether the derived DoH endpoint should be user-visible (read-only row in TUN preferences) before this ships, or documented only. Default: documented only, revisit on feedback.
