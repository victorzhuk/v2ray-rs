# Tasks

## 1. Loopback Proxy Hardening for Probes

- [x] 1.1 Update `node_proxy_client` in `crates/subscription/src/real_delay.rs` to use `reqwest::Proxy::all(...)` and `.no_proxy()`, skip `detect_egress` for failed delay indices, and verify unit/loopback-CONNECT tests confirm `https://` egress and `availability.rs` checks tunnel through the node inbound even with `NO_PROXY=*`

## 2. Xray TUN Strict IPv6 DNS Capture & Route Helper Gate

- [x] 2.1 Install IPv6 port-53 capture rules (`pref 8999`) whenever `v6_rules && cfg.capture_dns` in `crates/netctl/src/net.rs` and verify netctl tests cover `--strict --capture-dns` without `--v6`
- [x] 2.2 Require effective UID 0 for `--force-legacy` in `crates/netctl/src/main.rs` before opening netlink and verify CLI tests reject non-root `--force-legacy` calls

## 3. Subscription Redirect, TLS, and Imported Profile Trust Boundaries

- [x] 3.1 Add a custom `reqwest::redirect::Policy` in `crates/subscription/src/fetch.rs` rejecting redirects to loopback, link-local, RFC 1918, unique-local, unspecified IP literals, or `localhost`, and verify redirect unit tests block private targets while allowing public hops
- [x] 3.2 Change `Subscription::new_from_url` and missing-field serde default of `use_imported_profile` to `false` in `crates/core/src/models/subscription.rs` (keeping `true` for file/pasted JSON imports), add a per-subscription profile opt-in toggle and import notice in `crates/ui/src/subscriptions.rs`, and verify serde round-trip and default tests pass
- [x] 3.3 Reject `allowInsecure` TLS nodes in `crates/subscription/src/json_import.rs` and `crates/subscription/src/parser.rs` with surfaced warnings, and verify mixed-source import tests keep valid nodes while reporting skipped `allowInsecure` entries
- [x] 3.4 Update `crates/subscription/src/json_import.rs` and `crates/core/src/config/writer.rs` to ignore `"protocol": "dns"` outbounds, fold tautological `"network": "tcp,udp"`, drop multi-axis or unsupported Xray routing rules whole with warnings, and validate all imported DNS servers, DNS rules, host overrides, and routing rules, verifying unit tests cover QUIC block rules, `dns-out`, and invalid imported CIDRs/domains

## 4. Geodata Tag Validation & Config Preview Redaction

- [x] 4.1 Validate rule-set tags against `[a-z0-9][a-z0-9_-]*` in `crates/core/src/geodata.rs` and `crates/core/src/config/singbox.rs` before constructing `.srs` paths or URLs, and verify unit tests reject `../`, `/`, and uppercase/invalid inputs after canonicalization
- [x] 4.2 Redact sensitive transport headers (`Authorization`, `Proxy-Authorization`, `Cookie`, `Set-Cookie`, `X-Api-Key`) and tokenized DoH/H3 server URLs in `crates/core/src/config/redact.rs`, preserve `reveal_raw` across `Refresh` in `crates/ui/src/config_preview.rs`, and verify redaction unit tests cover headers, query strings, custom DoH paths, and `public_key` visibility

## 5. Setuid Wrapper, Privilege Grant, and Packaging Hardening

- [x] 5.1 Verify `getresuid`/`getresgid` after privilege drop and replace the env denylist with an explicit session-variable allowlist in `crates/run/src/main.rs`, verifying unit tests cover environment sanitization
- [x] 5.2 Provision system user `v2ray-rs-bypass` when `v2ray-rs-run` is present in `crates/process/src/privilege.rs`, ship `v2ray-rs-run` as `0755` in `pkg/archlinux/{,bin/}PKGBUILD` while moving `chown root:v2ray-rs` + `chmod 4750` to `v2ray-rs.install`, and fix helper asset/mode handling in `scripts/install-appimage.sh` and `scripts/install.sh`, verifying `bash -n` and `privilege.rs` tests pass

## 6. Cross-Crate Integration Verification

- [x] 6.1 Run `cargo test --workspace` with bounded timeouts and `openspec validate harden-security-and-import-trust --strict` to confirm all security and import-trust contracts pass together

## Workflow follow-up

- Update `CHANGELOG.md` `[Unreleased]` when landing user-visible import-profile and security behavior changes.
- Archive the change after implementation and verification complete.
