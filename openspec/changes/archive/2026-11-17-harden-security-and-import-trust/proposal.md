# Proposal

## Why

Active security and trust-boundary defects can leak the host's real IP during HTTPS egress and geo-availability probes, leak IPv6 port-53 DNS on LAN under strict xray TUN without an IPv6 tunnel address, silently accept untrusted DNS/routing overrides or `allowInsecure` TLS nodes from remote subscription URLs, and leave privilege/path/redaction boundaries weaker than specified.

## What Changes

- Route both HTTP and HTTPS probe requests through each node's loopback proxy inbound with ambient proxy env vars disabled, and skip egress requests for nodes whose delay probe failed.
- Capture unmarked IPv6 port-53 DNS in strict xray TUN even when no IPv6 tunnel address is configured, and restrict `--force-legacy` route-helper invocations to effective UID 0.
- **BREAKING**: Default `use_imported_profile` to `false` for URL subscriptions (requiring explicit user opt-in in UI while keeping `true` for local file and pasted JSON imports), reject `allowInsecure` TLS nodes at import time with a warning, drop multi-axis or unsupported Xray JSON routing rules whole instead of widening them, ignore `"protocol": "dns"` outbounds during import, and validate all imported DNS and routing structures before persistence and config generation.
- Block SSRF redirects from public subscription URLs to loopback, link-local, private, unique-local, or unspecified targets, and validate GeoIP/GeoSite rule-set tags before constructing `.srs` cache paths or URLs.
- Harden `v2ray-rs-run`, `pkexec` grant, Arch `PKGBUILD`s, and installer scripts so archives never ship setuid binaries, `v2ray-rs-bypass` is provisioned during privileged setup, `v2ray-rs-run` verifies UID/GID drop and allowlists environment variables, and unprivileged installs handle helper modes accurately.
- Expand config-preview redaction to sensitive custom transport headers and tokenized DoH/H3 URLs, and preserve the reveal toggle state across Refresh.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `real-delay-latency-test`: Tunnel HTTPS egress probes through the node loopback proxy, ignore ambient proxy env vars, and skip egress probes on failed delay nodes.
- `geo-availability-probe`: Tunnel HTTPS service checks through the node loopback proxy and ignore ambient proxy env vars.
- `tun-mode`: Capture IPv6 port-53 DNS under strict xray TUN without an IPv6 tunnel address and require root for `--force-legacy` helper recovery.
- `tun-process-bypass`: Verify UID/GID drop and allowlist environment in `v2ray-rs-run`, create `v2ray-rs-bypass` during privileged setup, and ship `0755` wrapper binaries in package archives.
- `subscription-update`: Block private-target HTTP redirects, require explicit opt-in for URL-imported profiles, reject `allowInsecure` imports, drop lossy multi-condition Xray rules, and validate imported profile DNS/routing inputs.
- `geodata-management`: Validate rule-set tags before `.srs` path or URL interpolation.
- `config-preview`: Redact sensitive transport headers and DoH/H3 URL path/query tokens, and preserve reveal state on Refresh.

## Impact

Affects `v2ray-rs-core` (`models/subscription.rs`, `geodata.rs`, `config/writer.rs`, `config/singbox.rs`, `config/redact.rs`), `v2ray-rs-subscription` (`real_delay.rs`, `availability.rs`, `fetch.rs`, `parser.rs`, `json_import.rs`), `v2ray-rs-netctl` (`main.rs`, `net.rs`), `v2ray-rs-process` (`privilege.rs`), `v2ray-rs-run` (`main.rs`), `v2ray-rs-ui` (`subscriptions.rs`, `config_preview.rs`), `pkg/archlinux/{,bin/}PKGBUILD`, `scripts/install.sh`, and `scripts/install-appimage.sh`. No new external crate dependencies required.
