# Proposal: harden-security-packaging-and-architecture

## Why

Broader review of the subscription parser/fetcher, system tray, CI/install scripts, and workspace architecture uncovered two high-severity security bugs, packaging/CI gaps, dead startup I/O, and architectural drift:

- **Trojan URI security bypass** (`crates/subscription/src/parser.rs:377-384`): `parse_trojan` only calls `parse_url_tls` when `security=tls` or `security=reality`. Standard Trojan URIs frequently omit `security` because TLS is implicit (`trojan://pw@host:443?sni=cdn.example.com&allowInsecure=1`). Those URIs fall into the `_` arm, bypassing `allowInsecure` rejection and silently discarding `sni`, `alpn`, and `fp`.
- **Subscription SSRF filter gap** (`crates/subscription/src/fetch.rs:158-182`): `ip_is_private_or_local` checks `v4.is_unspecified()` (`0.0.0.0` only), allowing `0.0.0.1`-`0.255.255.255` (`0.0.0.0/8`), which Linux routes to loopback (`127.0.0.1`), plus IPv4 multicast (`224.0.0.0/4`), reserved (`240.0.0.0/4`), and IPv6 multicast (`ff00::/8`).
- **CI and installer drift**: `.github/workflows/ci.yml:38` omits `v2ray-rs-antidpi/privileged-tests` from the privileged-test check; `scripts/install.sh:452-471` prints post-delete `setcap -r` commands for `$PREFIX/bin/*` files it just unlinked while omitting `/usr/local/lib/v2ray-rs/*`; `scripts/install-appimage.sh:485-520` and `docs/RELEASE.md:10, 184-189` omit `v2ray-rs-antidpi` from AppImage bundle/relocation instructions.
- **Dead tray startup I/O** (`crates/tray/src/tray.rs:63-78, 335-376`, `icons.rs:30-98`): `AppTray::icon_name()` unconditionally returns `""` (forcing `icon_pixmap()`), yet every launch still creates a `TempDir`, writes SVGs, and spawns `gtk-update-icon-cache -f -t`.
- **Workspace architecture drift and god-module growth**: `openspec/specs/workspace-architecture/spec.md` still specifies an 8-crate layout while the repo now has 9 crates (`antidpi`), `AGENTS.md` claims `crates/subscription` depends only on `v2ray-rs-core` when `crates/subscription/Cargo.toml` also depends on `v2ray-rs-process` (for observatory/real-delay probing), `openspec/specs/geo-availability-probe/spec.md` still has a placeholder Purpose section, and `crates/process` has grown three 4,500-5,300 LOC monolithic files (`session/mod.rs` 5,312 LOC, `manager.rs` 4,816 LOC, `privilege.rs` 4,516 LOC).

## What Changes

- Fix `parse_trojan` (`crates/subscription/src/parser.rs`) to route `None | Some("") | Some("tls") | Some("reality")` through `parse_url_tls` (defaulting `server_name` to `address` when `sni` is absent) and reject unknown `security` values.
- Harden `ip_is_private_or_local` (`crates/subscription/src/fetch.rs`) to block IPv4 `0.0.0.0/8`, multicast (`224.0.0.0/4`), reserved (`240.0.0.0/4`), and IPv6 multicast (`ff00::/8`).
- Add `v2ray-rs-antidpi/privileged-tests` to `.github/workflows/ci.yml:38`, fix `scripts/install.sh --uninstall` cleanup output for relocated `/usr/local/lib/v2ray-rs/*`, and update `scripts/install-appimage.sh` and `docs/RELEASE.md` to document bundled `v2ray-rs-antidpi`.
- Remove dead `setup_icon_theme` / `install_icons` / `gtk-update-icon-cache` startup execution from `crates/tray` while `icon_name()` renders exclusively through `icon_pixmap()`.
- Reconcile `workspace-architecture` spec and `AGENTS.md` to the 9-crate DAG (external-binary-only `antidpi` boundary, explicit `subscription -> process` dependency), replace the placeholder Purpose in `openspec/specs/geo-availability-probe/spec.md`, and decompose `crates/process` monoliths (`session/mod.rs`, `manager.rs`, `privilege.rs`) into focused submodules.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `subscription-import`: Trojan URI parsing enforces `parse_url_tls` and `allowInsecure` rejection when `security` is omitted; subscription HTTP fetch blocks `0.0.0.0/8`, multicast, and reserved IP ranges.
- `system-tray`: Pixmap-only tray startup avoids creating unused temp icon theme directories or spawning `gtk-update-icon-cache`.
- `release-packaging`: CI compiles `v2ray-rs-antidpi/privileged-tests`; installers and release docs accurately cover `v2ray-rs-antidpi` and relocated helper cleanup.
- `workspace-architecture`: Canonical 9-crate workspace DAG (including external-binary `antidpi` and `subscription -> process` boundary) and modular decomposition of `crates/process`.

## Impact

- **Modified crates**: `crates/subscription` (`parser.rs`, `fetch.rs`), `crates/tray` (`tray.rs`, `icons.rs`), `crates/process` (module decomposition of `session/`, `manager/`, `privilege/`).
- **Modified CI/scripts/docs**: `.github/workflows/ci.yml`, `scripts/install.sh`, `scripts/install-appimage.sh`, `docs/RELEASE.md`, `AGENTS.md`, `openspec/specs/geo-availability-probe/spec.md`.
- **Dependencies**: None new.
