# Proposal

## Why

When DNS is enabled on sing-box without TUN, `default_domain_resolver` selects the first IP-literal DNS server even when that server detours through the proxy outbound (`DnsConfig::default()` and the Cloudflare and Google presets), creating a circular dependency where dialing a hostname-addressed proxy node requires resolving its domain through the proxy itself. Every client connection deadlocks while `sing-box check` and inbound startup succeed.

## What Changes

- Restrict sing-box `default_domain_resolver` selection to IP-literal DNS servers that dial directly (`detour` is `None` or `"direct"`), falling back to the generated bootstrap resolver (`sys-dns-bootstrap`).
- Emit `sys-dns-bootstrap` whenever `default_domain_resolver` selects it (including when IP-literal servers exist only behind a proxy detour), preventing circular resolution on proxy outbounds, hostname-addressed direct DNS servers, and unpinned TUN nodes. The bootstrap is `type: "local"` without TUN and a direct-dialed DoH server at an IP literal under TUN, where the shipped self-contained-TUN contract forbids depending on the OS resolver. The `Effective DNS` summary reports whichever form the generated config carries.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `config-generator`: Require sing-box `default_domain_resolver` and DNS server `domain_resolver` fields to select a non-detoured IP-literal server or a generated local bootstrap resolver so proxy outbounds never depend on themselves to resolve their own server hostname.

## Impact

`crates/core/src/config/singbox.rs`, `crates/core/src/config/effective_dns.rs`, and `crates/core/tests/singbox_check.rs`. No settings schema changes or new dependencies.
