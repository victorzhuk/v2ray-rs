# Carry the host pin into the sing-box dial path and stop emitting an unstartable detour

## Why

`fix-tun-dns-bootstrap` fixed the xray side of the TUN name-resolution problem and left the sing-box side unexamined. Measured against sing-box 1.13.21, the same class of defect is present there, plus one that is worse than a deadlock.

**1. A DNS server detoured to "direct" produces a config the daemon refuses to start.** Generating the current shape for a server with `detour: "direct"` and running the real binary:

```
FATAL start service: start dns/udp[domestic]: detour to an empty direct outbound makes no sense
```

`sing-box check` accepts that configuration, so the whole test suite passes and the failure only appears when a user connects. The DNS server dialog offers "direct" for sing-box and has since the detour combo was added, so this is reachable by anyone who picks it. sing-box rejects the detour because a DNS server carrying none is not dispatched through the proxy chain to begin with; naming the empty direct outbound expresses nothing.

**2. The derived TUN plane drops the connect-time host pin.** Exactly the regression `fix-tun-dns-bootstrap` fixed for xray. `dns.hosts` is written into the effective settings before every connect, but the sing-box `hosts` server is built only on the path the DNS feature takes. With the feature off, the pin never reaches the config.

**3. Dial-time resolution does not consult the DNS rules, so the pin never applied to the proxy's own hostname even when it was emitted.** Measured with a `hosts` rule at index 0 matching the proxy hostname and `route.default_domain_resolver` naming a proxy-detoured server: the rule was ignored, the query went to the detoured server, and resolution hung for seven seconds before failing. The proxy's hostname was being resolved through the proxy being dialed, and the pin sitting in the same config could not help, because `default_domain_resolver` names one server and bypasses the rule engine.

## What Changes

- A detour of "direct" is no longer emitted as a `detour` field for sing-box. The field is emitted only for a proxy detour, which is the only value the backend can express. This changes a configuration that could not start into one that can.
- The `hosts` server and its DNS rule are emitted on the derived TUN path as well as the DNS-feature path, so the connect-time pin reaches every generated config.
- A proxy outbound whose server hostname is pinned carries `domain_resolver: "hosts"`, naming the pin as its own resolver. Dial-time resolution bypasses `dns.rules`, so this is the only way the pin reaches the dial. It needs no network at all and therefore cannot be circular. An unpinned node is left as it is rather than pointed at a server that answers NXDOMAIN on a miss.
- The sing-box integration tests gain a harness that starts the real daemon and fails on any `FATAL`, because `sing-box check` demonstrably accepts configurations that cannot run. The TUN inbound is stripped for those cases, since creating the device needs a capability the test runner does not hold.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `dns-configuration`: "DNS server detour per backend" corrected — sing-box expresses only a proxy detour, and a direct detour is expressed by the absence of the field.
- `config-generator`: "TUN mode DNS resolution is self-contained" extended to sing-box — the pin is emitted on the derived path, and a pinned proxy outbound names the pin as its dial resolver.

## Impact

- `crates/core/src/config/singbox.rs` — shared `hosts` server and rule, detour emission, per-outbound dial resolver.
- `crates/core/tests/singbox_check.rs` — startup harness.
- Behavior change: a profile whose DNS server was set to a "direct" detour previously produced a config that would not start, and now starts with that server undetoured. Configurations that already started are unaffected apart from gaining the pin.
