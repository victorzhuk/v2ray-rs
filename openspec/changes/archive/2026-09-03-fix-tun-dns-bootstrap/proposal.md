# Bootstrap DNS for the proxy hostname and stop the preset apply from aborting

## Why

Two defects make TUN mode unusable, both reproduced on a live install (xray 26.3.27, TUN on, DNS settings off, RU Bypass rules).

**1. Applying a DNS provider preset aborts the app.** `apply_dns_preset` succeeds, then the preset dialog re-syncs the strategy row with the `Rc<RefCell<AppSettings>>` still borrowed:

```rust
ctx_inner.strategy_row.set_selected(strategy_to_index(ctx_inner.state.borrow().dns.strategy));
```

The `Ref` temporary lives to the end of the statement, so it is still held while `set_selected` synchronously emits `notify::selected`; that handler takes `borrow_mut()` on the same cell. `BorrowMutError` panics inside a glib `extern "C"` trampoline, which cannot unwind, so the process aborts. Three `SIGABRT` coredumps carrying `RefCell already borrowed!` were captured on 2026-09-03. Every builtin preset carries `PreferIpv4` and `AdwComboRow` early-returns on an unchanged value, so the abort fires only when the user's current strategy is something else — the affected install has `ipv4_only`. The abort lands before the 300 ms settings debounce flushes, so the preset is never persisted and the app comes back with the old DNS.

**2. xray + TUN cannot resolve its own proxy hostname when the DNS feature is off.** `pin_node_addresses` resolves every hostname-addressed node through the OS resolver before the tunnel exists and writes the answers into `dns.hosts`, and `capture_dns` is gated on that pinning succeeding. But `dns.hosts` is only emitted by the DNS-enabled generation path; the derived TUN plane emits `servers` and `queryStrategy` only and silently drops the pins. Meanwhile every dialing outbound carries `sockopt.domainStrategy`, and the first routing rule sends all internal-resolver traffic to the first proxy outbound. Dialing the proxy therefore needs its own hostname resolved by a resolver that is only reachable through that same proxy:

```
[Error] app/dns: failed to retrieve response for <proxy host>. > Post "https://1.1.1.1/dns-query": context deadline exceeded
[Error] transport/internet: failed to resolve ip > app/dns: returning nil for domain <proxy host> > record not found
```

Because the pre-connect lookup succeeded, the interlock reports "pinned" and kernel-side DNS capture stays armed, so application DNS dies with it: captured `udp/53` reaches `dns-out`, the internal resolver, and the same deadlock.

Underneath both symptoms the xray DNS plane has no depth. The pin is a one-shot snapshot frozen for the process lifetime — a rotated server IP, a stale pin, or a failed pre-connect lookup puts the tunnel straight back into the deadlock with nothing to fall back on. sing-box has a bootstrap resolver (`sys-dns-bootstrap` plus `route.default_domain_resolver`); xray has none. `DnsServerConfig.detour` is discarded for xray, so the "domestic" server of every builtin preset is proxied anyway and the escape hatch the previous change documented ("users can restore split resolution by enabling DNS settings with a domestic server") does not exist. The fallback resolver appended when every server is domain-scoped is the same endpoint already in use, so one unreachable endpoint is a total outage.

## What Changes

- The DNS preferences page stops re-entering its own signal handlers: programmatic widget updates run under a suppression flag, and no `RefCell` borrow is held across a GTK setter. Applying a preset now also syncs the master enable switch, which `apply_dns_preset` turns on but the UI never reflected.
- `dns.hosts`, `disableCache` and `clientIp` are emitted on every xray TUN path, not only when the DNS feature is enabled, so the connect-time pin reaches the generated config.
- xray gains a bootstrap resolver: a pair of server objects sharing `tag: "dns-direct"` and a `domains` list scoped to the names that must resolve before the tunnel works — every hostname-addressed node and every hostname-addressed DNS server. Plain UDP is tried first and DoH second, because DoH on 443 is blocked on many of the networks this app exists to serve while UDP/53 is transparently intercepted on those same networks; neither transport is sufficient alone. Both carry `skipFallback`; only the last carries `finalQuery`, so the pair is tried in order and nothing falls back past it onto a resolver reachable only through the proxy being resolved. A routing rule at index 0 sends the tag through `direct`, which carries the fwmark the route helper's pref-9000 rule uses to reach the real default route. `dns.hosts` remains the first thing consulted; the bootstrap only answers when the pin is missing or stale. An IP-literal-only config emits neither.
- Static host overrides are filtered to the address family the query strategy will use, and a domain left with no usable address is dropped rather than emitted — an unusable `hosts` entry is an authoritative empty answer, not a fall-through to `servers`.
- The empty-server-list fallback stops reaching for the operating-system resolver under xray + TUN and uses the derived DoH endpoint instead. xray's local resolver form bypasses the outbound stack onto unmarked sockets, which is exactly what the route helper captures back into the tunnel. The `exclude_domains` split-horizon server keeps using it deliberately: those names exist only on the local resolver, and sending them to a public endpoint would not resolve them at all.
- xray honors a `direct` detour on a DNS server through the same `dns-direct` tag. The default stays "through the proxy", so the poisoning-resistance decision from `harden-tun-dns-resolution` is preserved and only an explicit user choice opts out. The DNS server dialog stops discarding the detour for xray.
- Excluded domains bind to the server detoured to `direct` when one exists, on both backends. The previous "first server without a detour" pick was the proxied default under TUN, so split-horizon names never actually resolved outside the tunnel.
- The connect-time pin carries every family and the xray generator keeps the one its query strategy uses; a node whose only override is of the other family counts as unpinned. The TUN runtime is built from the same effective settings the config was generated from, and `capture_dns` follows whether the config actually carries a pin for every hostname node.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `config-generator`: "TUN mode DNS resolution is self-contained" extended — static host overrides and the bootstrap resolver are part of the derived plane, and the routing rule order is specified relative to the `dns-internal` catch-all.
- `dns-configuration`: "Detour is sing-box only" replaced by a per-backend rule — sing-box emits `detour`, xray expresses a `direct` detour as a routing rule, v2ray still ignores it.
- `tun-mode`: new requirement covering proxy-hostname pinning and the DNS-capture interlock; "Privileged route helper for xray" extended with the port-53 capture rule the helper already installs.
- `dns-preferences-ui`: new requirement that programmatic widget updates never re-enter state-mutating handlers, and that applying a provider preset re-syncs the enable switch and the strategy row.

## Impact

- `crates/ui/src/preferences/dns.rs` — suppression guard, preset apply tail, detour retained for xray.
- `crates/ui/src/preferences/mod.rs` — `emit` no longer holds a borrow across the observer fan-out.
- `crates/core/src/config/v2ray.rs` — shared DNS tail, bootstrap server, detour and bootstrap routing rules, fallback de-duplication.
- `crates/ui/src/connection.rs` — effective settings into the TUN runtime, interlock tied to the emitted config and its address family.
- Behavior change: with TUN on and DNS off, the generated config gains a second DNS server and one routing rule. Users on an IP-addressed node see no change.
