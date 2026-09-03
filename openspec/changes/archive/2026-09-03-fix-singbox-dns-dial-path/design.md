# Design: the sing-box dial path

## Context

Everything below was measured against the installed sing-box 1.13.21, not inferred from documentation. Each finding is a config shape run through `sing-box check` and then through `sing-box run`.

| Shape | `check` | `run` |
| --- | --- | --- |
| DNS server with `detour: "direct"`, direct outbound is `{"type":"direct","tag":"direct"}` | passes | **FATAL** "detour to an empty direct outbound makes no sense" |
| Same, but the direct outbound carries any field | passes | starts |
| `route.default_domain_resolver` absent, a `dns` section present, outbound addressed by hostname | **fails** | — |
| No `dns` section at all, outbound addressed by hostname | passes | starts |
| `default_domain_resolver` as a bare tag, as an object, or naming the `hosts` server | passes | starts |
| `domain_resolver` on a `shadowsocks` outbound, bare tag or object | passes | starts |
| `type: "local"` as the dial resolver | passes | starts |

Two behavioural measurements matter more than the schema:

- **Dial-time resolution bypasses `dns.rules`.** With `{"domain":["ss.example.com"],"server":"hosts"}` at rule index 0 and `default_domain_resolver` naming a server detoured through the proxy, resolving that same proxy's hostname hung for 7s and failed with `context canceled`. The pin was present in the config and was not consulted.
- **`default_domain_resolver: "hosts"` does answer at dial time, and NXDOMAINs on a miss.** A domain absent from `predefined` produced `lookup failed for other.example.org: NXDOMAIN` rather than falling through to another server.

The first says the pin can only reach a dial by being named. The second says naming it globally is unsafe.

## Goals / Non-Goals

- Goal: every generated sing-box configuration starts. A shape that `check` accepts but the daemon rejects is a defect regardless of whether a user has hit it.
- Goal: a pinned proxy hostname resolves from the pin, with no network access and no dependence on the tunnel's state.
- Non-goal: changing the derived resolver endpoint or the poisoning-resistance decision from `harden-tun-dns-resolution`.
- Non-goal: a sing-box bootstrap resolver equivalent to xray's `dns-direct` pair. sing-box already has `sys-dns-bootstrap` for DNS-server hostnames, and the pin covers proxy hostnames. Adding a third mechanism would need the direct-egress question below settled first.

## Decisions

- **A direct detour is expressed by omitting the field.** sing-box's own error states the reason: a DNS server that carries no detour is not dispatched through the proxy chain, so detouring it to an empty direct outbound expresses nothing. Emitting the field for a proxy detour only is therefore both the minimal change and the one the backend asks for.
- **The pin is named per outbound, not globally.** `domain_resolver: "hosts"` goes on a proxy outbound only when that node's hostname actually appears in `dns.hosts`. A pinned node then resolves with no network at all, which is strictly stronger than any bootstrap: it cannot be poisoned, cannot be circular, and does not care whether the tunnel is up. An unpinned node keeps the existing `default_domain_resolver`, because pointing it at `hosts` would turn a slow lookup into an immediate NXDOMAIN.
- **`route.default_domain_resolver` is left alone.** It is still a proxy-detoured server on the derived path, which is still circular for an unpinned hostname node. The pin covers every node the app itself connects, since the connect path pins before generating. Fixing the unpinned case needs a resolver that provably egresses outside the tunnel, and that is the open question below.
- **The test harness starts the daemon.** `sing-box check` accepted the unstartable detour shape, so schema validation alone is not evidence that a configuration works. The startup harness reproduces the FATAL on the old code and passes on the new one; that negative control is what makes it worth its runtime.
- **Host overrides are not family-filtered for sing-box.** xray needs the filter because it answers a `hosts` hit authoritatively against a single-family `queryStrategy`. sing-box takes both families in `predefined` and applies `strategy` afterwards, so filtering at generation would cost it the fallback family on a single-stack network. Left as it is deliberately.

## Risks / Trade-offs

- [Omitting the detour makes a "direct" server follow the default outbound instead of egressing directly] → it would mean split-horizon names still resolve proxy-side, which is the behaviour today; what changes is that the configuration starts at all. See the open question.
- [A stale pin points a proxy outbound at a dead address] → the dial fails fast against a wrong address rather than hanging on a circular lookup, and the pin is rewritten on every connect.
- [The startup harness adds about six seconds to the suite] → it is the only thing standing between this class of defect and a release, and it is scoped to four cases.

## Settled after implementation

Both questions this design opened were answered against the v1.13.21 source.

- **A DNS server with no detour dials directly from sing-box's own process.** `common/dialer/dialer.go` `NewWithOptions` branches on the detour tag: set, it pins that one outbound; unset, it builds a plain socket dialer honoring only that server's own dial fields. DNS servers reach it through `dns/transport_dialer.go`. Neither branch consults `route.rules`. So omitting the field is not merely the shape that starts, it is the shape that egresses outside the proxy, and excluded domains genuinely resolve outside the tunnel. The startup guard exists precisely because detouring to an option-less direct outbound would re-derive the same dialer.
- **Dial-time resolution bypasses `dns.rules` by construction, in both forms.** `dns/router.go` `Lookup` queries the named transport directly whenever one is configured and skips the rule-matching loop; `route.default_domain_resolver` and a per-outbound `domain_resolver` behave identically. Configuring no resolver is the only way to reach the rule engine, and that is already fatal in 1.13.21 and removed in 1.14.0. Naming the pin per outbound is therefore the only mechanism available, not merely the one chosen.

The `hosts` server's two failure modes were also confirmed: a domain it does not hold returns NXDOMAIN, and one whose addresses the strategy filters out returns NOERROR with no answers. Neither falls through, which is why it is named per pinned outbound rather than globally.

## Open Questions

- Whether `route.default_domain_resolver` should become a `local` server on the derived path, covering unpinned hostname nodes. `local` has no automatic self-exclusion under `auto_route`; the documented loop prevention is `route.auto_detect_interface`, which binds outbound connections to the default interface and which this generator already emits whenever TUN is on. That suggests the amplifying loop `fix-tun-dns-bootstrap` rejected for xray does not apply here, but it has not been exercised under a live tunnel.
