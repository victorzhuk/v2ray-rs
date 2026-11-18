# release-packaging Specification

## Purpose
Defines reproducible version resolution for CI and manual release builds, Linux package build and runtime metadata, workspace-complete Makefile targets, and documentation parity across the 8-crate repository.

## Requirements

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

### Requirement: Installers require verified downloads
Both release installers SHALL verify the payload SHA-256 before extraction, execution, or prefix modification.

#### Scenario: Verification unavailable or unsuccessful
- **WHEN** the verifier is missing, hashing fails, or the expected digest is unavailable, malformed, or mismatched
- **THEN** installation SHALL exit unsuccessfully and preserve the existing installation, including in quiet mode

#### Scenario: Verified alternative version
- **WHEN** an explicitly selected version has a matching release checksum
- **THEN** both installers SHALL accept its verified payload

### Requirement: Arch helper privileges require restricted access
Arch installation and upgrade SHALL verify root ownership, the v2ray-rs group, and restricted execute permissions before enabling helper privileges.

#### Scenario: Provisioning fails halfway
- **WHEN** either helper fails ownership, permission, privilege, or final-state verification
- **THEN** the hook SHALL report failure and remove any partially applied privileges, reporting any failed cleanup

#### Scenario: Successful provisioning
- **WHEN** provisioning succeeds
- **THEN** netctl SHALL have mode 0750 and CAP_NET_ADMIN and the wrapper SHALL have mode 4750, both owned by root:v2ray-rs

### Requirement: Arch removal preserves system identities
Package removal SHALL preserve the bypass account and access group.

#### Scenario: Existing account and group
- **WHEN** the package is installed and removed on a system with existing service identities
- **THEN** their UID, GID, and existing memberships SHALL remain intact

### Requirement: AppImage installer TUN guidance names the tarball installer and tolerates unset USER
When the AppImage installer prints post-install TUN setup instructions, the system-wide install command SHALL reference the release tarball installer URL (`install.sh`) rather than the AppImage installer URL, and group-enrollment guidance SHALL succeed under strict unset-variable shell execution when `USER` is unset in the environment.

#### Scenario: Fresh install in a minimal environment without USER or v2ray-rs group
- **WHEN** `install-appimage.sh` runs with `USER` unset on a system where the `v2ray-rs` group does not exist
- **THEN** the installer SHALL exit with status `0` after printing TUN instructions whose system-wide install command points to `.../install.sh`

### Requirement: Release workflow verifies PKGBUILD versions and stamps release asset checksums
Before publishing a tagged release, the release workflow SHALL verify that both Arch `PKGBUILD` `pkgver` values match the workspace and tag version, and the `PKGBUILD` and `PKGBUILD-bin` files uploaded as GitHub Release assets SHALL carry verified SHA-256 digests instead of `SKIP` placeholders.

#### Scenario: Tag push with stale PKGBUILD version fails before publication
- **WHEN** `release.yml` runs on tag `v0.20.4` while `pkg/archlinux/PKGBUILD` or `pkg/archlinux/bin/PKGBUILD` still declares `pkgver=0.20.3`
- **THEN** version verification SHALL fail the build before any GitHub Release or AUR push runs

#### Scenario: Published PKGBUILD release assets contain concrete SHA-256 digests
- **WHEN** the `release` job stages `release-assets/PKGBUILD` and `release-assets/PKGBUILD-bin` for a tag release
- **THEN** `PKGBUILD-bin` SHALL contain the 64-hex SHA-256 of the built `v2ray-rs-x86_64-linux.tar.gz` and `PKGBUILD` SHALL contain the 64-hex SHA-256 of the tagged source archive, with neither file retaining `sha256sums=('SKIP')`
