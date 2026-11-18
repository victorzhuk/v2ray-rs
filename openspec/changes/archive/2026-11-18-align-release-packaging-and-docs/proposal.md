# Proposal

## Why

Release workflows, Linux package recipes, `Makefile` targets, and repository docs have drifted behind the 8-crate workspace and current build requirements. Manual `workflow_dispatch` runs of `release.yml` fail in `gen-installer-appimage.sh`, clean-chroot Arch `makepkg` lacks `protoc`, `cargo-deb` references missing icon paths and omits TUN helpers, `Makefile` omits three workspace crates, and contributor/architecture docs describe superseded module paths and dependencies.

## What Changes

- Resolve the application version from `Cargo.toml` (exposed via job outputs) in `.github/workflows/release.yml` so `appimage` installer generation succeeds on both tag pushes and branch `workflow_dispatch` runs.
- Add `'protobuf'` to `makedepends` in `pkg/archlinux/PKGBUILD` so clean-chroot Arch builds satisfy `prost-build` (`protoc`) in `crates/core/build.rs`.
- Repair `[package.metadata.deb]` in `crates/ui/Cargo.toml` to point at existing icon assets, bundle `v2ray-rs-netctl` and `v2ray-rs-run` with `0755` archive modes, and declare runtime library dependencies (`libgtk-4-1`, `libadwaita-1-0`, `libdbus-1-3`, `libcap2`).
- Extend `Makefile` to cover all 8 workspace crates (`netctl`, `run`, `tun-session` alongside `core`, `subscription`, `process`, `tray`, `ui`) and pass `--workspace` in `check`, `check-all`, `doc`, and `fix`.
- Update `docs/ARCHITECTURE.md`, `docs/RELEASE.md`, and `CONTRIBUTING.md` to match the current crate graph (`tray -> core` without `process`), `process::session` headless supervision and connect-time DNS pinning, `WorkspaceStore`, `subscription_blobs/`, `v2ray-rs.install` release asset, `rust-version = "1.93"` / `1.93.1` toolchain, `protobuf-compiler` setup, bounded test invocations, and `.agents/skills/` paths.

## Capabilities

### New Capabilities
- `release-packaging`: Defines reproducible version resolution across tag and `workflow_dispatch` release runs, distribution package build/runtime metadata (`PKGBUILD`, `cargo-deb`), full 8-crate `Makefile` targets, and architecture/release documentation parity.

### Modified Capabilities
None.

## Impact

- `.github/workflows/release.yml`
- `pkg/archlinux/PKGBUILD`
- `crates/ui/Cargo.toml`
- `Makefile`
- `docs/ARCHITECTURE.md`, `docs/RELEASE.md`, `CONTRIBUTING.md`
