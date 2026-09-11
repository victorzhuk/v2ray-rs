# Design: one source for each xray field

## Context

- `harden_tun_outbounds` (`crates/core/src/config/v2ray.rs:143`) sets `settings.domainStrategy = "UseIP"` on every `freedom` outbound under xray TUN; `apply_tun_domain_strategy` (`crates/core/src/config/xray.rs:64`) sets `streamSettings.sockopt.domainStrategy` on every dialing outbound from the DNS strategy (`Ipv6Only`/`PreferIpv6` → `UseIPv6`, else `UseIPv4`).
- Xray-core ≥ v26.9.8 copies a non-`AsIs` freedom strategy over `sockopt.domainStrategy`; before that, freedom resolved with its own strategy and the dialer applied `sockopt`. Both resolve through the built-in resolver, which is all the TUN hardening needs.
- The TLS builder (`v2ray.rs:378`) is shared by the v2ray and xray generators and writes `allowInsecure` unconditionally. v2fly/v2ray still accepts it.
- `crates/core/tests/xray_check.rs` runs `xray run -test` on generated configs and skips when no binary is installed.

## Goals / Non-Goals

**Goals:**
- Generated xray configs load on 26.1.13 through 26.9.9 without deprecation warnings.
- A node xray cannot express fails alone, with its name in the error.

**Non-Goals:**
- Certificate pinning (`pinnedPeerCertSha256`) as a replacement for disabled verification — it needs a certificate hash the subscription formats do not carry.
- Renaming still-accepted legacy keys (`streamSettings.network` → `method`, dns-outbound `address` → `rewriteAddress`); they load without warnings today.

## Decisions

- **Drop `settings.domainStrategy`, keep `sockopt`.** `sockopt.domainStrategy` already routes freedom's lookups to the built-in resolver on every supported xray version, so removing the settings field loses nothing and removes the override. Alternative rejected: writing `"UseIP"` into `sockopt` for freedom — it would re-introduce the dual-stack behavior against an IPv4-only strategy that 26.9.8 made visible.
- **Refuse at generation, per candidate.** `write_config` already fails a single candidate and the planner moves on (`connection.rs:139`). A TLS node with `verify == false` returns a generation error for xray: `"node '<node>' disables certificate verification, which backend xray does not support; enable verification or use sing-box"`. The check covers every node in the generated config, so a rule-pinned via-node with verification off fails the candidate too — the same outcome xray's own start check produced, now with the node named. Alternative rejected: silently emitting a verifying config — the connection would fail at the handshake with a TLS error that hides the cause.
- **Backend-aware TLS builder.** The builder takes the backend and emits `allowInsecure` only for v2ray.
- **Guard in the integration test, not at runtime.** `xray_check` asserts no stdout/stderr line contains `deprecated`; runtime detection would only produce a log line nobody reads. Xray-core 26.9.9 also prints feature-level notices for the Shadowsocks protocol and the WebSocket transport; those come from the node the user picked, not from a setting the generator emits, so the guard allowlists exactly those two feature names and still fails on every other deprecation line.

## Risks / Trade-offs

- [A user relying on a verification-off node under xray loses it] → it already fails the whole start on 26.6.22+ and on 26.3.27 after its built-in cut-off date; the new error names the node and the way out.
- [Deprecation guard makes the test suite fail on a future xray upgrade] → intended; the suite skips without a binary, so CI without xray is unaffected.

## Migration Plan

Generator-only change; the next Connect regenerates the config. Rollback is a revert.
