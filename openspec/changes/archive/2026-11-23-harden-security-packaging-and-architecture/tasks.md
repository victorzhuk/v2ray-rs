# Tasks: harden-security-packaging-and-architecture

## 1. Subscription security hardening (`crates/subscription`)

- [x] 1.1 Update `parse_trojan` (`crates/subscription/src/parser.rs:377-384`) to route `None | Some("") | Some("tls") | Some("reality")` through `parse_url_tls(&params)?` (defaulting `server_name` to `Some(address.clone())` when `sni` is absent) and reject unknown `security` values.
- [x] 1.2 Add Trojan parser tests for omitted `security` preserving `sni`/`alpn`/`fp`, rejecting `allowInsecure=1`, and rejecting unknown `security` values.
- [x] 1.3 Update `ip_is_private_or_local` (`crates/subscription/src/fetch.rs:158-182`) to block IPv4 `0.0.0.0/8`, multicast (`224.0.0.0/4`), reserved (`240.0.0.0/4`), IPv6 multicast (`ff00::/8`), and NAT64 well-known prefix (`64:ff9b::/96`) embedded local IPv4 addresses; add unit tests.

## 2. CI, installer scripts, and release docs

- [x] 2.1 Add `v2ray-rs-antidpi/privileged-tests` to `.github/workflows/ci.yml:38`.
- [x] 2.2 Update `scripts/install.sh:452-471` (`uninstall`) to stop printing `setcap -r` for already-deleted `$PREFIX/bin/*` files and report cleanup instructions for relocated `/usr/local/lib/v2ray-rs/{v2ray-rs-netctl,v2ray-rs-antidpi}`; update `scripts/test-installers.sh`.
- [x] 2.3 Update `scripts/install-appimage.sh:485-520` and `docs/RELEASE.md:10, 184-189` to document that `v2ray-rs-antidpi` is bundled alongside `v2ray-rs-netctl` and relocated by the shared grant.

## 3. System tray startup hygiene (`crates/tray`)

- [x] 3.1 Remove dead `setup_icon_theme()` / `install_icons()` / `gtk-update-icon-cache` startup execution from `TrayService::spawn` and `TrayService::spawn_with_data_dir` (`crates/tray/src/tray.rs:335-376`, `icons.rs:30-98`) while keeping pixmap rendering and explicit `--install-icons` behavior intact.

## 4. Architecture reconciliation and `crates/process` decomposition

- [x] 4.1 Update `AGENTS.md` to accurately document `crates/subscription`'s `v2ray-rs-process` dependency (observatory/real-delay probing) and replace the placeholder Purpose in `openspec/specs/geo-availability-probe/spec.md`.
- [x] 4.2 Decompose `crates/process/src/session/mod.rs` (5,312 LOC), `crates/process/src/manager.rs` (4,816 LOC), and `crates/process/src/privilege.rs` (4,516 LOC) into focused submodules (`session/`, `manager/`, `privilege/`) preserving all public exports (execute after `fix-process-session-and-privilege-safety` lands).

## 5. Verification

- [x] 5.1 Run `timeout 5m cargo test -p v2ray-rs-subscription -p v2ray-rs-tray -p v2ray-rs-process -- --test-threads=4`.
- [x] 5.2 Run `make test-installers`, `make fmt`, `make clippy`, and `cargo check --workspace --all-targets --features v2ray-rs-process/privileged-tests,v2ray-rs-netctl/privileged-tests,v2ray-rs-antidpi/privileged-tests`.
  - `make test-installers` and `make fmt` pass. The changed crates pass both `cargo clippy -p v2ray-rs-subscription -p v2ray-rs-tray -p v2ray-rs-process --all-targets --all-features -- -D warnings` and `cargo check` with the same selection; `crates/antidpi` lint findings were fixed and its package is clippy-clean with `privileged-tests`.
  - `make clippy` and `cargo check --workspace` still stop at `crates/ui` compile errors (`app/mod.rs` `ProcessEvent` arity, `preferences/antidpi.rs` types). Those files are uncommitted work of `fix-ui-grant-cleanup-and-ux-bugs`, and the user chose to leave them to that change.