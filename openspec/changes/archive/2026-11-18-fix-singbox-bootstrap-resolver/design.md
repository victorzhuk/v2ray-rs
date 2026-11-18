# Design

## Context

See `proposal.md` for motivation. In `crates/core/src/config/singbox.rs`:
- `default_domain_resolver_tag` currently picks `settings.dns.servers.iter().find(|s| s.address.parse::<IpAddr>().is_ok())` regardless of `s.detour`.
- `build_dns` emits `{"tag": "sys-dns-bootstrap", "type": "local"}` only when `!plan.has_ip_literal_server()`.
- When `tun.enabled` is `false`, `session::pins_enabled` is `false`, so `build_outbounds` emits no `domain_resolver: "dns-hosts"` on hostname-addressed proxy outbounds. Sing-box therefore resolves the proxy outbound's `server` domain through `route.default_domain_resolver`. If that resolver carries `"detour": "proxy-0"`, sing-box enters a dial loop (`proxy-0` -> `remote` -> `proxy-0`) on every connection, and hostname-addressed direct DNS servers (`server["domain_resolver"]`) inherit the same proxy-detoured resolver.

## Goals / Non-Goals

**Goals:**
- Guarantee `default_domain_resolver_tag` never returns a DNS server tag that detours through a proxy outbound.
- Keep `dns.servers` and `route.default_domain_resolver` consistent so `sing-box check` and live traffic succeed across all 8 built-in DNS presets, `DnsConfig::default()`, custom single-server proxy DoH setups, and unpinned TUN sessions.

**Non-Goals:**
- Enabling pre-connect OS DNS pinning (`dns-hosts`) for non-TUN connections.

## Decisions

1. **Require direct dialability for IP-literal bootstrap selection.**
   Evaluate candidate bootstrap servers against the same detour condition `build_dns` uses when emitting `"detour": first_proxy_tag`: a configured DNS server qualifies as `default_domain_resolver` only when `s.address.parse::<IpAddr>().is_ok() && (s.detour.is_none() || s.detours_direct())` (matching `ResolvedDnsPlan` semantics). When no configured server is both IP-literal and non-proxy-detoured, return `Some(BOOTSTRAP_RESOLVER_TAG)` whenever `!settings.dns.servers.is_empty()`.
   *Alternative considered:* Falling back to `plan.first_tag()` when all IP-literal servers detour through the proxy. Rejected because `plan.first_tag()` itself detours through `proxy-0` and reproduces the recursion deadlock.

2. **Couple `BOOTSTRAP_RESOLVER_TAG` emission directly to `default_domain_resolver_tag`.**
   In `build_dns`, append the bootstrap server whenever `default_domain_resolver_tag(settings).as_deref() == Some(BOOTSTRAP_RESOLVER_TAG)` instead of checking `!plan.has_ip_literal_server()`. `dns.final` remains `plan.first_tag()`, so proxied DNS queries still default to `"remote"`. Without TUN the bootstrap is `{"tag": BOOTSTRAP_RESOLVER_TAG, "type": "local"}`. With TUN it is a direct-dialed DoH server at an IP literal, `{"tag": BOOTSTRAP_RESOLVER_TAG, "type": "https", "server": "1.1.1.1", "server_port": 443, "path": "/dns-query"}`, because the shipped `TUN mode DNS resolution is self-contained` requirement forbids OS-resolver dependence under TUN, and a typed DNS server without `detour` uses the direct dialer (the forbidden `detour: "direct"` names an empty direct outbound). `EffectiveDns` mirrors both entries through the same predicate.

## Risks / Trade-offs

- **[Risk] Direct DoH bootstrap unreachable (1.1.1.1:443 blocked, IPv6-only network without NAT64, captive portal)** -> Hostname-addressed proxies cannot resolve and the connection fails loudly instead of leaking through a poisoned OS resolver, matching the xray TUN bootstrap precedent. Recovery is a connect-time host pin, a direct IP-literal DNS server, or TUN off. sing-box has no ordered inter-server fallback, so a secondary bootstrap server would be dead config.
