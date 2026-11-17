# Proposal

## Why

Generated backend configs, DNS settings, geodata refresh passes, and proxy URI/JSON parsers currently break or misroute under valid inputs: `sing-box` fails startup when DNS rules reference `GeoSite` tags without `route.rule_set` entries, `xray` `exclude_domains` mutates a user's unrestricted DNS server into a domain-only `skipFallback` server, `.srs` rule-sets never update once cached, custom UDP DNS ports and bare IPv6 literals format invalidly, and URI/ping code mishandles IPv6 brackets, `xhttp` transports, non-443 Trojan TLS, and `via_node` detours from disabled subscriptions.

## What Changes

- Register `route.rule_set` entries (lowercased `geosite-{category}`) in `sing-box` configs for active custom and auto-derived DNS `GeoSite` rules, include those tags in `singbox_rule_set_tags`, and re-download existing `.srs` rule-sets during scheduled and manual geodata refreshes.
- Preserve unrestricted user DNS servers when attaching `tun.exclude_domains` in `v2ray`/`xray` by emitting a dedicated domain-scoped `skipFallback` entry, preserve `DomainKeyword` and `DomainFull` matchers in `sing-box` auto-derived DNS rules, and keep `effective_dns` aligned with generated resolvers.
- Bracket bare IPv6 literals and honor custom UDP ports in `DnsProtocol::server_address`, enforce IPv4/IPv6 family checks on FakeIP CIDRs, reject reserved internal DNS server tags, validate `DnsRuleMatch` and `HostOverride` values, and run `DnsConfig` validation on TUN-derived DNS fields even when `dns.enabled` is `false`.
- Normalize bracketed IPv6 host literals across VLESS/VMess/Shadowsocks/Trojan/JSON parsers and `tcp_ping`, percent-decode VLESS/Trojan credentials, accept case-insensitive URI schemes, parse `xhttp`/`splithttp` URI transports while returning errors on unknown transports, keep Trojan TLS enabled by default on non-443 ports unless `security=none`, check VMess base64 payload bounds before decode and `u16` port conversion, and skip `verify == false` TLS nodes in Xray Real Delay probe configs.
- Exclude nodes belonging to disabled subscriptions when resolving `via_node` detour chains.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `config-generator`: Emit `sing-box` rule-set definitions for DNS `GeoSite` rules, keep unrestricted `v2ray`/`xray` DNS servers intact under `tun.exclude_domains`, preserve `DomainKeyword`/`DomainFull` in `sing-box` derived DNS rules, and skip `verify == false` nodes in Xray probe batches.
- `dns-configuration`: Format custom UDP ports and bracketed IPv6 literals in `server_address()`, tighten `DnsConfig` validation (FakeIP address family, reserved tags, rule/host validation, TUN-derived fields), and keep `effective_dns` exact under `exclude_domains`.
- `geodata-management`: Collect DNS `GeoSite` rule-set tags for `sing-box` and refresh already-cached `.srs` files on scheduled and manual updates.
- `subscription-update`: Parse `xhttp`/`splithttp` URI transports, reject unknown URI transports, normalize IPv6 literals in parsers and `tcp_ping`, fix Trojan non-443 default TLS, percent-decode credentials, and enforce case-insensitive schemes and VMess bounds.
- `connection-auto-resolve`: Refuse `via_node` detours that reference nodes inside disabled subscriptions.

## Impact

Affects `v2ray-rs-core` (`config/singbox.rs`, `config/v2ray.rs`, `config/effective_dns.rs`, `models/dns.rs`, `resolve.rs`, `geodata.rs`), `v2ray-rs-subscription` (`parser.rs`, `json_import.rs`, `ping.rs`, `real_delay.rs`), and `v2ray-rs-ui` (`geodata_service.rs`). No new external dependencies required.
