# Design: add-tun-exclude-routing

## Backend asymmetry

The two TUN backends differ in kind, not just effort:

- **sing-box** resolves each connection to its owning process via `/proc`, so
  `process_name` route rules match regardless of how the tool was launched
  (app-launched or already-running). Process, domain, and CIDR exclusion are all
  pure config.
- **xray** cannot match TUN-captured traffic by process — its `process` routing
  field only matches locally-dialled sockets, and TUN packets arrive as an
  inbound from the virtual device. So xray exclusion is destination-only here
  (CIDR + domain). Per-process bypass for xray is handled in the sequenced
  `add-tun-process-bypass` change via a dedicated UID, out of scope for this one.

## Why destination exclusion already bypasses the tunnel (xray)

`apply_tun_fwmark` stamps fwmark 255 on every non-blackhole xray outbound,
including `direct`/`freedom`. The route helper installs a policy rule (pref 9000)
that sends fwmark-255 packets to the main table. So routing an excluded
destination to the `direct` outbound makes xray dial it directly, and that dial
bypasses the tunnel with no route-helper change. CIDR rules were previously not
emitted for xray; this change emits them (and domain rules) when TUN is enabled.

## DNS handling (avoid hijack leaks)

- **sing-box** has a flat `dns.rules` array: excluded domains get a parallel
  `{ domain_suffix: [...], server: <direct resolver> }` rule and excluded
  processes a `{ process_name: [...], server: <direct resolver> }` rule, so their
  lookups skip DNS hijack. The direct resolver is the first non-detour server,
  falling back to a plain resolver when none is configured.
- **xray/v2ray** DNS has no rules array — domain→resolver affinity is expressed
  per server as a `domains: [...]` field. Excluded domains are appended to the
  direct/domestic DNS server's `domains` list so they resolve outside the tunnel.

## Rule ordering

Exclusion rules are *prepended* ahead of the user's routing rules in both
generators so they take precedence. The generators already receive
`&AppSettings`; `build_route` (sing-box) and `build_routing` (xray) gain access to
`settings.tun` to read the exclusion lists.

## Field semantics & validation

- `exclude_processes`: process basenames (e.g. `cloudflared`); rejected if empty
  or containing a path separator. sing-box matches the basename; xray ignores the
  list (surfaced in the UI note).
- `exclude_domains`: domain suffixes validated by the existing
  `validate_domain_pattern`. sing-box uses `domain_suffix`; xray uses the `domain`
  field, which performs substring/suffix matching given sniffing is enabled on the
  TUN inbound (it is).
- `exclude_routes`: unchanged CIDR validation; now consumed by xray too.

## Out of scope

xray per-process bypass, the setuid launcher, and the "Run with bypass" UI action
are deferred to `add-tun-process-bypass`. Catching already-running tools on xray
is intentionally not built — sing-box covers it natively, and the xray mechanism
would require a privileged `/proc` reconciler.
