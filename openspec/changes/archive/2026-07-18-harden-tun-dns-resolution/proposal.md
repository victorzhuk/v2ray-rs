# Harden DNS resolution in TUN mode

## Why

TUN mode currently rides entirely on the operating system's resolver whenever the DNS feature is off (`dns.enabled = false`, the default — and the observed live config). The generated xray TUN config has no `dns` section at all, so three resolution paths all end at the ISP resolver, which poisons answers for blocked domains on the networks this app targets:

1. Routing: `domainStrategy: "IPIfNonMatch"` resolves sniffed domains to match `geoip:*` rules. A poisoned answer inside RU address space makes a blocked foreign domain match `geoip:ru → direct`, so the connection egresses directly and is killed by DPI — the observed `proxy/tun: connection reset by peer` / `connection was refused` log lines (per Xray semantics these surface the real upstream dial failing), and the user-visible symptom: Claude Code API streams dropping mid-session.
2. Direct dials: with sniffing `destOverride` the original IP is discarded and the `freedom` outbound (default `domainStrategy: "AsIs"`) re-resolves the hostname through the OS resolver at dial time — same poisoned answers.
3. Application DNS: apps' own plaintext UDP:53 to the LAN resolver flows through the tunnel's `192.168.0.0/16 → direct` rule untouched — poisoned and plaintext-visible.

sing-box has the same hole: the `hijack-dns` rule and the whole `dns` block are gated on `dns.enabled`, so `tun.dns_hijack = "hijack"` (the user's setting) silently does nothing. Additionally, xray TUN requires Xray-core ≥ v26.1.13 (the `tun` inbound did not exist in v25.x); today an older binary only fails with an opaque config-test error.

## What Changes

- TUN mode gets a self-contained DNS plane, independent of `dns.enabled`: when TUN is on and DNS settings are off, generators derive a minimal trusted DNS config (DoH to 1.1.1.1 routed through the proxy) instead of omitting the DNS section. User-configured DNS keeps working and is hardened the same way.
- xray, when TUN is enabled:
  - emit `dns.tag` (e.g. `"dns-internal"`) plus a routing rule `{"inboundTag": ["dns-internal"], "outboundTag": <first proxy>}` ahead of user rules, so every query made by xray's built-in resolver travels through the proxy, immune to local poisoning;
  - the `freedom` direct outbound gains `"domainStrategy": "UseIP"` so direct dials resolve through the built-in (clean) resolver instead of the OS resolver;
  - `tun.dns_hijack = Hijack` now applies to xray: emit a `{"protocol": "dns", "tag": "dns-out"}` outbound and a `{"network": "udp", "port": 53, "outboundTag": "dns-out"}` routing rule so TUN-captured application DNS is answered by the built-in resolver; `Native`/`Disabled` skip the rule.
- sing-box, when TUN is enabled and DNS settings are off: derive the same minimal DNS block (`dns.servers` DoH with proxy detour, `dns.final`, `route.default_domain_resolver`) so the existing `hijack-dns` route rule becomes reachable and honors `dns_hijack`.
- Connect preflight: starting TUN with an xray older than v26.1.13 fails with a clear versioned error instead of the raw config-test output. Versions 26.1.13 through 26.6.22 additionally get a log advisory: they carry an upstream TUN crash on quickly-closed connections (`panic: Net: Unknown address type.`, Xray-core #6364, fixed in 26.6.27) that drops the tunnel for a crash-restart window each time it fires. The companion change `suppress-tun-latency-pings` removes the app's own trigger for that panic.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `config-generator`: new requirement "TUN mode DNS resolution is self-contained"; "Generate xray TUN inbound" extended with the DNS-hijack and internal-resolver routing shape.
- `tun-mode`: "TUN mode availability per backend" gains the xray minimum-version gate.

## Impact

- `crates/core/src/config/v2ray.rs` / `xray.rs` — DNS derivation, `dns.tag` + routing rule, freedom `domainStrategy`, dns-out hijack outbound/rule.
- `crates/core/src/config/singbox.rs` — minimal derived DNS when settings DNS is off and TUN is on.
- `crates/core/src/backend.rs` / connect flow — xray version parse + TUN gate.
- Trade-off documented in design: with derived DNS, RU-domain resolution also goes through the proxy (geo-CDN affinity may shift); users can restore split resolution by enabling DNS settings with a domestic server.
