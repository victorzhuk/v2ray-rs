# Tasks

## 1. DNS Model Formatting & Validation (`v2ray-rs-core`)

- [x] 1.1 Update `DnsProtocol::server_address` in `crates/core/src/models/dns.rs` to honor custom UDP ports and bracket bare IPv6 literals across UDP/TCP/DoH/DoT/DoQ/H3, and verify unit tests cover IPv4/IPv6 + default/custom port combinations
- [x] 1.2 Tighten `DnsConfig::validate` and `ConfigWriter::validate_runtime_inputs` in `crates/core/src/models/dns.rs` and `crates/core/src/config/writer.rs` to check IPv4 vs IPv6 FakeIP CIDR families, reject reserved server tags (`hosts`, `fakeip`, `sys-dns-bootstrap`, `tun-dns-fallback`), validate `DnsRuleMatch` and `HostOverride` entries, and validate `hosts`/`client_subnet` when `tun.enabled && !dns.enabled`, verifying unit tests cover each rejection case

## 2. Backend Config Generators & Geodata Rule-Set Sync

- [x] 2.1 Register lowercased `geosite-<category>` `route.rule_set` entries for active custom and auto-derived DNS `GeoSite` rules and preserve `DomainKeyword`/`DomainFull` in auto-derived DNS rules in `crates/core/src/config/singbox.rs`, verifying unit tests check emitted `route.rule_set`, `experimental.cache_file`, and `dns.rules`
- [x] 2.2 Emit a dedicated domain-scoped `skipFallback` server entry for `tun.exclude_domains` when the chosen direct server is unrestricted in `crates/core/src/config/v2ray.rs` and mirror the resolver list in `crates/core/src/config/effective_dns.rs`, verifying generator and `effective_dns` parity tests pass
- [x] 2.3 Include active DNS `GeoSite` tags in `singbox_rule_set_tags` and re-download cached `.srs` files on scheduled and manual refresh passes in `crates/ui/src/geodata_service.rs` and `crates/core/src/geodata.rs`, verifying tests confirm existing `.srs` files are refreshed on manual/scheduled runs
- [x] 2.4 Omit `verify == false` TLS nodes from Xray probe configs in `crates/subscription/src/real_delay.rs` while preserving batch indices, and disallow `via_node` resolution from disabled subscriptions in `crates/core/src/resolve.rs`, verifying unit tests cover both cases

## 3. Subscription URI/JSON Parsers & IPv6 Ping (`v2ray-rs-subscription`)

- [x] 3.1 Strip `[...]` brackets on IPv6 hosts in `crates/subscription/src/parser.rs` and `crates/subscription/src/json_import.rs`, reject bracketed hosts in `ProxyNode::validate`, and use `SocketAddr` formatting for IPv6 targets in `crates/subscription/src/ping.rs`, verifying IPv6 URI parse + loopback TCP ping tests pass
- [x] 3.2 Parse `xhttp`/`splithttp` transports into `TransportSettings::Xhttp` and return `Err` on unknown transport types in `crates/subscription/src/parser.rs`, and verify VLESS/Trojan/VMess transport tests cover `xhttp` and rejected `httpupgrade`/`kcp`
- [x] 3.3 Fix case-insensitive scheme stripping (`vmess://`, `ss://`), percent-decode VLESS/Trojan userinfo, default Trojan TLS to enabled on non-443 ports unless `security=none`, check VMess base64 encoded length before decode, and enforce `u16::try_from` port bounds in `crates/subscription/src/parser.rs`, verifying parser unit tests cover every case

## 4. Integration Verification

- [x] 4.1 Run `cargo test -p v2ray-rs-core -p v2ray-rs-subscription` with bounded timeouts and `openspec validate fix-config-dns-and-ipv6 --strict` to verify all config, DNS, geodata, and parser contracts pass

## Workflow follow-up

- Update `CHANGELOG.md` `[Unreleased]` for DNS/IPv6/XHTTP URI fixes when landing implementation.
- Archive the change once implementation and verification complete.
