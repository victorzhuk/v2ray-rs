# Design

## Context

See `proposal.md` for motivation. Over recent architecture and TUN-hardening changes, the workspace grew to 8 crates (`core` using `prost-build` in `build.rs`, `tun-session`, `netctl`, `run`, `subscription`, `process` with `session/`, `tray` decoupled from `process`, and decomposed `ui`), while `release.yml`, `pkg/archlinux/PKGBUILD`, `crates/ui/Cargo.toml` `[package.metadata.deb]`, `Makefile`, and repository docs retained older assumptions.

## Goals / Non-Goals

**Goals:**
- Make `release.yml` `appimage` job succeed on `workflow_dispatch` as well as `v*` tag pushes.
- Make `pkg/archlinux/PKGBUILD` buildable in clean Arch chroots (`extra-x86_64-build`) and `cargo-deb` metadata valid against current icons and helper binaries.
- Bring `Makefile` up to full 8-crate workspace parity.
- Synchronize `docs/ARCHITECTURE.md`, `docs/RELEASE.md`, and `CONTRIBUTING.md` with the actual codebase.

**Non-Goals:**
- Changing runtime Rust code in any crate.
- Adding privilege-granting `postinst` `setcap`/`chmod 4750` maintainer scripts to `.deb` packages without operator consent (keep archive modes `755` consistent with `tun-process-bypass` PKGBUILD policy).

## Decisions

### 1. Share resolved `version` output in `release.yml`
- **Choice:** Expose `outputs: version: ${{ steps.meta.outputs.version }}` on the `tarball` job (or extract `[workspace.package].version` via `sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1` directly in `appimage` when `GITHUB_REF != refs/tags/v*`) so `scripts/gen-installer-appimage.sh` always receives a valid `X.Y.Z` semver string on both tag pushes and `workflow_dispatch`.
- **Why:** Computing version in `appimage` with the same `if [[ "${GITHUB_REF}" == refs/tags/v* ]]` logic as `tarball` keeps `tarball` and `appimage` running **in parallel** without serializing `appimage` behind `tarball`.

### 2. Clean-chroot `PKGBUILD` and `[package.metadata.deb]`
- **Choice:**
  - Add `'protobuf'` to `makedepends` in `pkg/archlinux/PKGBUILD` (note: `glib2` is already in `depends`, and `crates/ui` has no `build.rs` calling `glib-compile-resources`, whereas `crates/core/build.rs` invokes `prost_build` requiring `/usr/bin/protoc` from `protobuf`).
  - In `crates/ui/Cargo.toml` `[package.metadata.deb]`, update `depends` to `"libgtk-4-1, libadwaita-1-0, libdbus-1-3, libcap2"`, include `target/release/v2ray-rs-netctl` and `target/release/v2ray-rs-run` at `usr/bin/` (`755`), and replace the broken `assets/v2ray-rs.svg` / `assets/v2ray-rs.png` entries with `crates/ui/icons/hicolor/scalable/apps/com.github.v2ray-rs.svg` -> `usr/share/icons/hicolor/scalable/apps/com.github.v2ray-rs.svg` (`644`).

### 3. `Makefile` 8-crate parity and `--workspace`
- **Choice:** Add `NETCTL = v2ray-rs-netctl`, `RUN = v2ray-rs-run`, `TUN_SESSION = v2ray-rs-tun-session`, corresponding `build-netctl`, `build-run`, `build-tun-session`, `test-netctl`, `test-run`, `test-tun-session` targets, and add `--workspace` to `cargo check`, `cargo clippy` (`--workspace --all-targets`), `cargo doc --workspace --no-deps`, and `cargo clippy --fix --workspace`.

### 4. Documentation updates
- **Choice:** Update only verified factual drift in `docs/ARCHITECTURE.md` (`tray` dependencies, `process::session` supervisor & `session/pins.rs`, `ui` module layout & `WorkspaceStore`, `subscription_blobs/`), `docs/RELEASE.md` (`v2ray-rs.install` artifact row), and `CONTRIBUTING.md` (`Rust 1.93+`, `protobuf-compiler`, bounded test invocations without false race-detector labels, `persistence/` directory, `openspec/specs/<capability>/spec.md`, `.agents/skills/`).

## Risks / Trade-offs

- **[Risk] `cargo-deb` asset paths resolve relative to workspace root vs package manifest** -> Mitigation: Verify `cargo-deb` path resolution (workspace-relative when workspace `Cargo.toml` is present) during implementation via `cargo metadata` / `cargo deb --no-build` dry-check if installed, or exact documented `cargo-deb` workspace asset rules.
