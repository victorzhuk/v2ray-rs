# Design: harden-security-packaging-and-architecture

## Context

This change addresses cross-cutting security, distribution, startup hygiene, and architectural maintainability gaps outside the anti-DPI engine and UI regression fixes.

## Decisions

### 1. Trojan TLS parsing and SSRF IP hardening (`crates/subscription`)
- In `parse_trojan` (`parser.rs:377-384`), match:
  - `Some("none")` -> `None`
  - `None | Some("") | Some("tls") | Some("reality")` -> call `parse_url_tls(&params)?` (which rejects `allowInsecure` and extracts `sni`, `alpn`, `fp`, REALITY params), then default `tls.server_name` to `Some(address.clone())` when `sni` was omitted.
  - `Some(other)` -> return `Err(ParseError::InvalidField(...))`.
- In `ip_is_private_or_local` (`fetch.rs:158-182`):
  - IPv4: block `octets[0] == 0` (`0.0.0.0/8`, routes to loopback on Linux), `v4.is_multicast()` (`224.0.0.0/4`), and `octets[0] >= 240` (`240.0.0.0/4`), in addition to existing loopback/private/link-local/broadcast/CGNAT/benchmarking checks.
  - IPv6: block `v6.is_multicast()` (`ff00::/8`) and inspect embedded IPv4 addresses for `64:ff9b::/96` (NAT64 well-known prefix) in addition to IPv4-mapped `::ffff:0:0/96`.

### 2. CI, installer, and release doc parity
- Add `v2ray-rs-antidpi/privileged-tests` to `.github/workflows/ci.yml:38` alongside `v2ray-rs-process/privileged-tests,v2ray-rs-netctl/privileged-tests`.
- In `scripts/install.sh:452-471` (`uninstall`), remove the post-delete `sudo setcap -r $PREFIX/bin/...` hints for binaries already unlinked by lines 452-454, and print cleanup guidance for relocated copies under `/usr/local/lib/v2ray-rs/{v2ray-rs-netctl,v2ray-rs-antidpi}` (matching `install-appimage.sh:374-376`).
- Update `scripts/install-appimage.sh:485-520` (`print_tun_instructions`) and `docs/RELEASE.md:10, 184-189` to document that the AppImage bundles both `v2ray-rs-netctl` and `v2ray-rs-antidpi` and relocates both via the shared grant.

### 3. Remove dead tray icon-theme startup I/O (`crates/tray`)
- `AppTray::icon_name()` unconditionally returns `String::new()` so all StatusNotifierItem hosts use `icon_pixmap()`.
- Remove automatic `setup_icon_theme()` / `install_icons()` / `gtk-update-icon-cache` calls from `TrayService::spawn` and `TrayService::spawn_with_data_dir` (keep `--install-icons` CLI flag working if explicitly invoked by desktop packaging, without touching temp dirs on every GUI launch).

### 4. Reconcile 9-crate architecture and decompose `crates/process` monoliths
- Update `workspace-architecture` spec and `AGENTS.md` to record:
  - 9 workspace crates (`core`, `tun-session`, `netctl`, `run`, `antidpi`, `subscription`, `process`, `tray`, `ui`) with `v2ray-rs-antidpi` invoked strictly as an external binary (no Cargo dependency from `process` or `ui`).
  - `v2ray-rs-subscription`'s existing dependency on `v2ray-rs-process` for observatory/real-delay backend probing.
- Decompose the three >4,500 LOC files in `crates/process/src/` into focused submodules (keeping public exports unchanged):
  - `session/`: split `mod.rs` (5,312 LOC) into supervisor loop, failover/cleanup helpers, runtime builders, and tests.
  - `manager/`: split `manager.rs` (4,816 LOC) into core lifecycle/spawn, TUN/anti-DPI orchestration, log/advisory forwarding, and tests.
  - `privilege/`: split `privilege.rs` (4,516 LOC) into capability/ACL/mount probes, grant planning, elevated script generation/verification, and tests.

## Risks / Trade-offs

- **Rejecting unknown `security` values in Trojan URIs**: Malformed third-party links with typos in `security=` will now report a parse error instead of silently ignoring all TLS parameters. Standard Trojan links (omitted `security`, `security=tls`, `security=reality`, `security=none`) continue to parse and now preserve `sni`/`alpn`/`fp`.
- **Refactoring `crates/process` modules**: Should execute after `fix-process-session-and-privilege-safety` lands so behavior fixes and structural splits do not create merge churn.
