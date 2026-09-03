# Design: a bootstrap path for the proxy hostname

## Context

Verified against the live install (xray 26.3.27, TUN on, DNS settings off, RU Bypass rules) and three `SIGABRT` coredumps from 2026-09-03:

- The generated config's `dns` block is `{"tag":"dns-internal","queryStrategy":"UseIPv4","servers":["https://1.1.1.1/dns-query"]}` — no `hosts`, although the connected node is addressed by hostname and the pre-connect pin succeeded.
- `routing.rules[0]` is `{"inboundTag":["dns-internal"],"outboundTag":"<first proxy>"}`, and every dialing outbound carries `sockopt.domainStrategy` plus `sockopt.mark: 255`. Resolving the proxy hostname therefore requires the proxy.
- The route helper installs the escape hatch at pref 9000 (`fwmark 255` → `main`) and captures unmarked port-53 traffic at pref 8999 into the tunnel table. Anything dispatched through an xray outbound carries the mark and escapes; anything xray resolves outside its outbound stack does not.
- xray's DNS server object at 26.3.27 carries a per-server `tag`, read out of the binary itself:
  `Address "json:\"address\""; ClientIP; Port; SkipFallback "json:\"skipFallback\""; Domains; ExpectedIPs; ExpectIPs; QueryStrategy; Tag "json:\"tag\""; TimeoutMs; DisableCache; ServeStale; ServeExpiredTTL; FinalQuery "json:\"finalQuery\""; UnexpectedIPs`.
  A tagged server's queries are stamped with that tag instead of `dns.tag`, so routing can address one server without matching on its destination. There is no `detour` field — a detour has to be expressed this way.
- `AdwComboRow::set_selected` emits `notify::selected` synchronously. A `RefCell` borrow held across it re-enters the handler and panics; the panic crosses a glib `extern "C"` frame and aborts.
- The target shape passes `xray run -test` on 26.3.27 (`Configuration OK.`): `hosts` with an address array, a `domains`-scoped server carrying `tag`, `skipFallback` and `finalQuery`, and an `inboundTag` routing rule for that tag.

## Measured on the affected network

Run against the real binary through a `freedom` outbound carrying `mark: 255`, i.e. outside the tunnel:

- `https://1.1.1.1/` — HTTP `000` after 1.02s. DoH on 443 to public resolvers is blocked. The same 1s reset appears inside xray as `Post "https://1.1.1.1/dns-query": io: read/write on closed pipe`, and moving the endpoint to `1.0.0.1` changes nothing.
- `udp/53` to `1.1.1.1`, `8.8.8.8`, `77.88.8.8` and `193.233.112.67` all answer, identically and correctly, in tens of milliseconds.
- `udp/53` to `192.0.2.1` — TEST-NET-1, which routes nowhere — also answers, correctly, in 4ms. Every UDP/53 packet is answered by the network regardless of destination address. The four "different" resolvers above were never reached.
- `udp/5353` to `192.0.2.1` times out after 4s, so the interception is port-scoped and a genuinely unreachable first entry does fall through to the second.

The consequence for this design: UDP/53 is available but its answers are the network's, not the resolver's; DoH is trustworthy but unreachable. The bootstrap is scoped to the proxy hostname and DNS-server hostnames precisely so this only matters for those names, and a poisoned answer for the proxy host fails closed at the TLS handshake rather than redirecting traffic.

## Goals / Non-Goals

- Goal: the proxy hostname is always resolvable without the proxy, by two independent mechanisms (the pin, and a resolver reachable outside the tunnel).
- Goal: no `RefCell` borrow is ever alive across a GTK setter on the DNS page, and programmatic updates do not re-enter handlers.
- Goal: the internal resolver keeps traversing the proxy by default; poisoning resistance is not traded away.
- Non-goal: changing the resolver endpoint (`https://1.1.1.1/dns-query`) or the `IPIfNonMatch` routing strategy.
- Non-goal: a general-purpose DNS failover engine. One distinct secondary is the bound.
- Non-goal: any change to sing-box's DNS plane, which already has `sys-dns-bootstrap` and `route.default_domain_resolver`.

## Decisions

- **The bootstrap is a pair of scoped server objects sharing a `dns-direct` tag, tried in transport order.** Because a tagged server stamps its own queries, the bootstrap does not need a distinct address to be routable, and one routing rule covers both:

  ```json
  {"tag":"dns-direct","address":"1.1.1.1","domains":["full:<proxy host>"],"skipFallback":true}
  {"tag":"dns-direct","address":"https://1.1.1.1/dns-query","domains":["full:<proxy host>"],
   "skipFallback":true,"finalQuery":true}
  ```

  with `{"type":"field","inboundTag":["dns-direct"],"outboundTag":"direct"}` at rule index 0. `direct` (freedom) carries `sockopt.mark: 255`, so the helper's pref-9000 rule takes it to the `main` table and it leaves on the real interface.

  Plain UDP comes first because the encrypted endpoint is unreachable on a large share of the networks this app exists to work on — measured, not assumed, below. DoH comes second because UDP/53 is transparently intercepted on those same networks, so an encrypted transport is the only one whose answer can be trusted where it is reachable. Neither transport is sufficient alone.

- **`finalQuery: true` on the last bootstrap entry only.** It stops the query from falling back onto the proxied resolvers — the deadlock again, several seconds slower. It must not be set on the first entry, which would end the query there and make the second unreachable.
- **Scope covers every name needed before the tunnel works**: each hostname-addressed node, and each hostname-addressed DNS server (four of the eight builtin presets use one — `dns.adguard.com`, `dns-family.adguard.com`, `dns.quad9.net`, `common.dot.dns.yandex.net`). An IP-literal-only config emits no bootstrap server and no `dns-direct` rule.
- **Alternative rejected — routing the bootstrap by destination IP.** It works, but forces the bootstrap onto an endpoint the primary does not use, breaks as soon as a direct server is addressed by hostname, and re-enters the resolver through `IPIfNonMatch`. The per-server tag is the mechanism xray actually provides.
- **Alternative rejected — bootstrapping through the system resolver.** `/etc/resolv.conf` on the affected host is `nameserver 127.0.0.1`. The stub is reachable, but its own upstream query leaves on an unmarked socket, which is exactly what the helper's pref-8999 rule pulls into the tunnel — an amplifying loop.
- **Alternative rejected — a `localhost` server.** `harden-tun-dns-resolution` already established that xray's local resolver form bypasses the outbound stack onto unmarked sockets. The empty-list fallback is therefore replaced by the derived DoH endpoint under TUN. The `exclude_domains` split-horizon server keeps `localhost` on purpose: those names exist nowhere else, so a public endpoint would not answer them, and the loop hazard is contained the way it already is — by listing the local resolver in `exclude_routes`.
- **`dns.hosts` stays the primary mechanism**, consulted before `servers` and costing no round trip. Emitting the hosts tail (`hosts`, `disableCache`, `clientIp`) becomes shared between the derived and the user-configured paths instead of living only in the latter, which is the actual regression.
- **The xray generator filters host overrides to the family its query strategy uses; the pin itself carries both.** A `hosts` entry that resolves to nothing usable is worse than no entry: it is an authoritative empty answer, not a fall-through. A domain left with no address of the right family is dropped so it degrades to a lookup. sing-box takes both families in its `hosts` server and dials through them in strategy order, so filtering at the pin would cost it the fallback family on a single-stack network.
- **A `direct` detour is honored for xray through the same tag**, opt-in only. The default remains the `dns-internal` catch-all to the proxy, so `harden-tun-dns-resolution`'s trade-off stands unless the user asks otherwise; what changes is that asking now works. Any detour value other than direct is not expressible and is logged and ignored.
- **Rule order.** The `dns-direct` rule goes at index 0. A plain-UDP direct server would otherwise be swallowed by the port-53 hijack rule, which carries no `inboundTag` constraint, and recurse through `dns-out` back into the resolver.
- **The UI uses a suppression flag, not deferred updates.** `Rc<Cell<bool>>` on the render context, set around every programmatic widget write, checked at the top of every state-mutating handler. Deferring through `glib::idle_add_local_once` would also break the borrow chain but leaves the widget briefly showing stale state and reorders against the settings debounce.
- **`capture_dns` follows the emitted config, not the lookup.** The interlock exists so DNS capture is never armed while the backend still needs the OS resolver; deriving it from the lookup result while the config drops the pin is what let the deadlock arm itself.

## Risks / Trade-offs

- [The DoH endpoint is unreachable outside the tunnel] → the pin still covers the normal case; both must fail before the deadlock returns, and `finalQuery` makes that failure immediate and visible instead of a four-second stall per name.
- [A second server object appears in configs that had one] → scoped with `skipFallback`, so it answers only for the bootstrap names; no effect on general resolution.
- [Honoring a `direct` detour re-opens local poisoning for that server] → opt-in, defaulted off, and visible in the server dialog.
- [Family filtering leaves xray with no usable pin and reports not-pinned] → that is the correct answer; capture stays off and the session still comes up, where today it comes up with DNS silently dead.

## Migration Plan

Config generation, one UI page, and the connect path. No settings-schema change, no persisted-state migration. Next Connect applies it. Rollback is a revert.

## Open Questions

- Whether split-horizon resolution for `tun.exclude_domains` should stop going through `localhost` entirely. It would need the local resolver's address, which means either reading `/etc/resolv.conf` or giving the local stub's uid a bypass rule — the mechanism exists (`RULE_PREF_BYPASS_UID`) but the plumbing is its own change.
- Whether the builtin presets should ship their `domestic` server with `detour: "direct"`. It is the actual product fix for split resolution and it also improves sing-box, but it changes behavior for existing profiles, so it wants its own change.
