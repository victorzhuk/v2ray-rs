## 1. xray generator

- [x] 1.1 Derive minimal DNS (`https://1.1.1.1/dns-query`) when `tun.enabled && !dns.enabled`; keep user DNS when enabled
- [x] 1.2 Emit `dns.tag: "dns-internal"` and prepend routing rule `{"inboundTag": ["dns-internal"], "outboundTag": <first proxy>}` whenever TUN is on and a dns section exists
- [x] 1.3 `freedom` direct outbound: add `"domainStrategy": "UseIP"` in TUN mode
- [x] 1.4 `dns_hijack = Hijack`: emit `{"protocol": "dns", "tag": "dns-out"}` outbound and `{"network": "udp", "port": 53, "outboundTag": "dns-out"}` rule (after the inboundTag rule, before exclusions/user rules); `Native`/`Disabled` omit it
- [x] 1.5 Tests: derived-DNS presence/absence matrix, rule ordering, hijack per mode, `UseIP` only in TUN mode

## 2. sing-box generator

- [x] 2.1 Gate DNS emission on `dns.enabled || tun.enabled`; derived path synthesizes the single DoH server with proxy detour, `final`, and `route.default_domain_resolver`
- [x] 2.2 Tests: TUN on + DNS off → dns block present, `hijack-dns` rule present when `dns_hijack = Hijack`, absent for `Native`/`Disabled`

## 3. xray TUN version gate

- [x] 3.1 Version check implemented as a connect preflight probe in `ProcessManager` (`xray version` output → semver triple, threshold 26.1.13); best-effort — unparsable output does not block
- [x] 3.2 Preflight (next to the CAP_NET_ADMIN gate): TUN + older xray → `TunBackendTooOld` naming installed and required versions
- [x] 3.3 Unit tests for the version parse, comparison, and the gate error path (fake pre-TUN xray script)
- [x] 3.4 Advisory for the upstream TUN panic range (26.1.13 ≤ v < 26.6.27, Xray-core #6364): warning line pushed into the log buffer and broadcast at TUN start; unit test with a fake 26.3.27 xray

## 4. Verification

- [x] 4.1 `cargo test --workspace` green; new `xray_check` integration test passes generated TUN configs (derived DNS, hijack, native) through the real `xray run -test`
- [x] 4.2 Live verification on the affected machine (2026-07-18): fixed config deployed into the running app's config path and respawned via the manager's crash recovery. Through the live TUN (DNS-off settings, RU Bypass rules, real node): blocked domains (claude.ai, instagram.com, rutracker.org) answered via proxy with no reset; api.anthropic.com 15/15 clean; 20 MB sustained stream uninterrupted; `dig @8.8.8.8` hijacked and answered with Anthropic's real IP; direct RU domains (ya.ru, habr.com) still fast. An active Claude Code session streamed through the tunnel throughout with no disconnect
