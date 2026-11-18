# Tasks

## 1. Non-Circular sing-box Domain Resolver Selection

- [x] 1.1 Update `default_domain_resolver_tag` and `build_dns` in `crates/core/src/config/singbox.rs` so only non-proxy-detoured IP-literal servers are chosen before falling back to `sys-dns-bootstrap`, and emit `sys-dns-bootstrap` whenever selected; verify unit tests in `crates/core/src/config/singbox.rs` cover `DnsConfig::default()`, all 8 `builtin_dns_presets()`, proxy-only IP DoH servers, and hostname-addressed direct DNS servers.
- [x] 1.2 Add live `sing-box check` and end-to-end loopback proxy traffic verification in `crates/core/tests/singbox_check.rs` (and/or `crates/process`) with `dns.enabled = true`, `tun.enabled = false`, `DnsConfig::default()`, and a hostname-addressed proxy outbound (`localhost`), verifying real proxied HTTP round-trip succeeds without resolver recursion.
