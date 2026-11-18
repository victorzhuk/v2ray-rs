# Spec Delta

## Purpose

Defines reproducible version resolution for CI and manual release builds, Linux package build and runtime metadata, workspace-complete Makefile targets, and documentation parity across the 8-crate repository.

## ADDED Requirements

### Requirement: Release workflow resolves version across tag and dispatch triggers
The GitHub release workflow SHALL derive the package version from a `v*` tag ref when triggered by a tag push and from `[workspace.package].version` in `Cargo.toml` when triggered via `workflow_dispatch`, passing that numeric `X.Y.Z` version to both the tarball and AppImage build jobs.

#### Scenario: Branch workflow_dispatch builds AppImage installer with Cargo version
- **WHEN** `release.yml` is triggered manually via `workflow_dispatch` on `master`
- **THEN** the `appimage` job SHALL pass the numeric `Cargo.toml` workspace version to `scripts/gen-installer-appimage.sh` instead of the branch name `"master"` and SHALL succeed without failing version validation

#### Scenario: Tag push uses tag version verified against Cargo.toml
- **WHEN** `release.yml` is triggered by pushing tag `v0.10.2`
- **THEN** both `tarball` and `appimage` jobs SHALL build artifacts stamped with `0.10.2` after verifying `Cargo.toml` matches

### Requirement: Distribution package metadata covers build tools, icons, and TUN helpers
Source and binary Linux package definitions SHALL declare required protobuf compiler build dependencies and valid asset paths: `pkg/archlinux/PKGBUILD` SHALL include `protobuf` in `makedepends`, and `[package.metadata.deb]` in `crates/ui/Cargo.toml` SHALL reference existing icon files, include `v2ray-rs-netctl` and `v2ray-rs-run` with mode `755`, and declare runtime shared-library dependencies.

#### Scenario: Arch source PKGBUILD declares protobuf makedepend
- **WHEN** `pkg/archlinux/PKGBUILD` is inspected or built in a clean Arch chroot
- **THEN** `makedepends` SHALL include `protobuf` alongside `rust` and `cargo` so `prost-build` finds `protoc`

#### Scenario: Debian package metadata references existing assets and helpers
- **WHEN** `[package.metadata.deb]` in `crates/ui/Cargo.toml` is validated against repository paths and release binaries
- **THEN** every static icon/desktop asset path SHALL exist in the repository and `v2ray-rs`, `v2ray-rs-netctl`, and `v2ray-rs-run` SHALL be mapped to `/usr/bin/` with mode `755`

### Requirement: Makefile covers all eight workspace crates
The root `Makefile` SHALL define per-crate `build-*` and `test-*` targets for all 8 workspace crates (`core`, `subscription`, `process`, `netctl`, `run`, `tun-session`, `tray`, `ui`) and SHALL pass `--workspace` on aggregate check, lint, fix, and documentation targets.

#### Scenario: Aggregate check and per-crate targets cover every crate
- **WHEN** a developer runs `make check`, `make check-all`, or per-crate `make build-<crate>` / `make test-<crate>` targets
- **THEN** `v2ray-rs-netctl`, `v2ray-rs-run`, and `v2ray-rs-tun-session` SHALL be checked, linted, and addressable alongside the other 5 crates

### Requirement: Repository documentation reflects workspace boundaries and release assets
`docs/ARCHITECTURE.md`, `docs/RELEASE.md`, and `CONTRIBUTING.md` SHALL accurately describe the 8-crate dependency graph (`tray` depending only on `core`), `process::session` supervision and connect-time DNS pinning, `WorkspaceStore` and `subscription_blobs/` storage, `v2ray-rs.install` release assets, the Rust 1.93 toolchain floor, `protobuf-compiler` setup, bounded test execution, and `.agents/skills/` paths.

#### Scenario: Architecture and contributor docs match Cargo manifests and file tree
- **WHEN** `docs/ARCHITECTURE.md`, `docs/RELEASE.md`, and `CONTRIBUTING.md` are cross-checked against `Cargo.toml`, `crates/*/Cargo.toml`, and `.github/workflows/release.yml`
- **THEN** all documented crate dependencies, module paths, Rust version floors, system build packages, and release artifact names SHALL match the repository
