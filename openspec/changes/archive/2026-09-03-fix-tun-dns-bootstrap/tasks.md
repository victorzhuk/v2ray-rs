## 1. DNS preferences page

- [x] 1.1 Add `enable_row` and a `Rc<Cell<bool>>` suppression flag to `DnsRenderCtx`; add a helper that raises the flag around a closure
- [x] 1.2 Every state-mutating handler on the page returns early while the flag is raised
- [x] 1.3 Preset apply reads values out of the borrow into locals, drops it, then drives `enable_row` and `strategy_row` under the guard
- [x] 1.4 `emit` binds the settings clone to a local before invoking the callback, so no borrow is alive during the observer fan-out
- [x] 1.5 The DNS server dialog keeps the detour value for xray instead of discarding it
- [x] 1.6 Test: a live borrow across `set_selected` under the guard leaves the handler silent instead of aborting (fails without the guard)

## 2. xray DNS plane

- [x] 2.1 Unify the derived and user-configured paths so `hosts`, `disableCache` and `clientIp` are emitted once, for both
- [x] 2.2 Filter host overrides to the family the query strategy uses; drop a domain left with no usable address and log it
- [x] 2.3 Emit the bootstrap pair — plain UDP then DoH, sharing `tag: "dns-direct"` and `skipFallback`, `finalQuery` on the last only, `domains` covering every hostname-addressed node and DNS server; omit both when every address is an IP literal
- [x] 2.4 Tag DNS servers whose detour is direct with `dns-direct`; every other value is the default route
- [x] 2.5 Emit `{"inboundTag":["dns-direct"],"outboundTag":"direct"}` at rule index 0 whenever a `dns-direct` server exists
- [x] 2.6 The empty-server-list fallback uses the derived DoH endpoint instead of the OS resolver under xray + TUN
- [x] 2.7 Excluded domains bind to the server detoured to `direct` when one exists, for both backends, instead of the first undetoured one
- [x] 2.8 Tests: hosts in the derived plane; bootstrap shape and scope; omitted for IP-literal-only configs; rule index 0 and ahead of the port-53 hijack; detour tagging; family filter including the drop case; v2ray unaffected

## 3. Connect path

- [x] 3.1 The pin carries every family; the capture interlock counts a node as pinned only when an override xray can answer with exists
- [x] 3.2 `build_tun_runtime` takes the effective settings, and `capture_dns` follows whether every hostname node is pinned in the settings the config was generated from
- [x] 3.3 Tests for the family filter and the interlock

## 4. Verification

- [x] 4.1 Workspace test run green; the xray TUN DNS tests are re-anchored on rule lookup by tag rather than index so the next insertion does not churn them
- [x] 4.2 `xray_check` gains derived-plane-with-pin, bootstrap, and direct-detour cases, passing through the real `xray run -test`
- [x] 4.3 Live A/B against the real binary on the affected network, dev profile, DNS off, no routing rules: old shape reproduces the deadlock (`[dns-internal -> proxy]`, `context deadline exceeded`, request fails); new shape resolves the proxy hostname in 33ms via `[dns-direct -> direct]` and the request succeeds
- [x] 4.4 Fallthrough verified with an unreachable first entry: both bootstrap entries queried over `direct`, no fallback onto the proxied resolvers, fails in 5s instead of deadlocking
- [ ] 4.5 Live check with the tunnel actually up (the dev run was contaminated by the production instance reclaiming table 2023, so the fwmark escape path is still only reasoned about, not measured)
- [ ] 4.6 Apply a provider preset from the `ipv4_only` strategy in the running app and confirm no abort, with the switch and strategy row updated
