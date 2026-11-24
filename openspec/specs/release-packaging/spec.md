# release-packaging Specification

## Purpose
Defines reproducible version resolution for CI and manual release builds, Linux package build and runtime metadata, workspace-complete Makefile targets, and documentation parity across the nine-crate repository.

## Requirements

### Requirement: Release workflow resolves version across tag and dispatch triggers
The GitHub release workflow SHALL derive the package version from a `v*` tag ref when triggered by a tag push and from `[workspace.package].version` in `Cargo.toml` when triggered via `workflow_dispatch`, passing that numeric `X.Y.Z` version to both the tarball and AppImage build jobs.

#### Scenario: Branch workflow_dispatch builds AppImage installer with Cargo version
- **WHEN** `release.yml` is triggered manually via `workflow_dispatch` on `master`
- **THEN** the `appimage` job SHALL pass the numeric `Cargo.toml` workspace version to `scripts/gen-installer-appimage.sh` instead of the branch name `"master"` and SHALL succeed without failing version validation

#### Scenario: Tag push uses tag version verified against Cargo.toml
- **WHEN** `release.yml` is triggered by pushing tag `v0.10.2`
- **THEN** both `tarball` and `appimage` jobs SHALL build artifacts stamped with `0.10.2` after verifying `Cargo.toml` matches

### Requirement: Arch source package builds and installs required binaries
`pkg/archlinux/PKGBUILD` SHALL include `protobuf` in `makedepends`.
It SHALL build `v2ray-rs-ui`, `v2ray-rs-netctl`, `v2ray-rs-run`, and `v2ray-rs-antidpi`.
It SHALL install each binary under `/usr/bin` with mode `755`.
The install hook tightens the privileged helpers.

#### Scenario: Arch source PKGBUILD declares protobuf makedepend
- **WHEN** `pkg/archlinux/PKGBUILD` is inspected or built in a clean Arch chroot
- **THEN** `makedepends` SHALL include `protobuf` alongside `rust` and `cargo` so `prost-build` finds `protoc`

#### Scenario: Arch source PKGBUILD builds and installs the anti-DPI helper
- **WHEN** `pkg/archlinux/PKGBUILD` is inspected
- **THEN** its `build()` SHALL include `-p v2ray-rs-antidpi` and `package()` SHALL install `v2ray-rs-antidpi` to `/usr/bin` with mode `755`

### Requirement: Debian package metadata covers assets and helpers
`[package.metadata.deb]` in `crates/ui/Cargo.toml` SHALL reference existing icon files.
It SHALL include `v2ray-rs-netctl`, `v2ray-rs-run`, and `v2ray-rs-antidpi` with mode `755`.
It SHALL declare runtime shared-library dependencies.

#### Scenario: Debian package metadata references existing assets and helpers
- **WHEN** `[package.metadata.deb]` in `crates/ui/Cargo.toml` is validated against repository paths and release binaries
- **THEN** every static icon/desktop asset path SHALL exist in the repository and `v2ray-rs`, `v2ray-rs-netctl`, `v2ray-rs-run`, and `v2ray-rs-antidpi` SHALL be mapped to `/usr/bin/` with mode `755`

### Requirement: Makefile covers all nine workspace crates
The root `Makefile` SHALL define per-crate `build-*` and `test-*` targets for all 9 workspace crates (`core`, `subscription`, `process`, `netctl`, `run`, `tun-session`, `tray`, `ui`, `antidpi`) and SHALL pass `--workspace` on aggregate check, lint, fix, and documentation targets. The `dist` target SHALL build the anti-DPI helper statically against musl alongside the other helpers and stage it as `bin/v2ray-rs-antidpi` in the release tree.

#### Scenario: Aggregate check and per-crate targets cover every crate
- **WHEN** a developer runs `make check`, `make check-all`, or per-crate `make build-<crate>` / `make test-<crate>` targets
- **THEN** `v2ray-rs-netctl`, `v2ray-rs-run`, `v2ray-rs-antidpi`, and `v2ray-rs-tun-session` SHALL be checked, linted, and addressable alongside the other 5 crates

#### Scenario: dist tarball carries the anti-DPI helper
- **WHEN** `make dist` completes and the tarball is listed
- **THEN** the staged release tree SHALL contain `bin/v2ray-rs-antidpi` beside the other binaries, and the tarball installer and uninstaller SHALL treat it as one of the installed binaries

### Requirement: Repository documentation reflects workspace boundaries and release assets
`docs/ARCHITECTURE.md`, `docs/RELEASE.md`, and `CONTRIBUTING.md` SHALL accurately describe the nine-crate workspace and direct Cargo dependencies. They SHALL distinguish Cargo dependencies from executable invocations. They SHALL describe `process::session` supervision and connect-time DNS pinning, `WorkspaceStore` and `subscription_blobs/` storage, `v2ray-rs.install` release assets, the Rust 1.93 toolchain floor, `protobuf-compiler` setup, bounded test execution, and `.agents/skills/` paths.

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
Arch installation and upgrade SHALL verify root ownership, the v2ray-rs group, and restricted execute permissions before enabling helper privileges. The anti-DPI helper SHALL be provisioned with `root:v2ray-rs` ownership, mode `0750`, and file capabilities `cap_net_admin,cap_net_raw+ep`, verified via `getcap` before the hook reports success.

#### Scenario: Provisioning fails halfway
- **WHEN** either helper — route helper, wrapper, or anti-DPI helper — fails ownership, permission, privilege, or final-state verification
- **THEN** the hook SHALL report failure and remove any partially applied privileges, including the anti-DPI capability, reporting any failed cleanup

#### Scenario: Successful provisioning
- **WHEN** provisioning succeeds
- **THEN** netctl SHALL have mode 0750 and CAP_NET_ADMIN, the wrapper SHALL have mode 4750, and the anti-DPI helper SHALL have mode 0750 and `cap_net_admin,cap_net_raw` — all owned by root:v2ray-rs

#### Scenario: Removal revokes anti-DPI capabilities
- **WHEN** the package is removed
- **THEN** the hook SHALL remove the file capabilities from the anti-DPI helper alongside the route helper's, without touching the retained group and bypass account

### Requirement: Arch removal preserves system identities
Package removal SHALL preserve the bypass account and access group.

#### Scenario: Existing account and group
- **WHEN** the package is installed and removed on a system with existing service identities
- **THEN** their UID, GID, and existing memberships SHALL remain intact

#### Scenario: System backend grants remain under administrator control
- **WHEN** an uninstall removes package-owned helpers
- **THEN** it SHALL explain that prior system-backend capability grants are not automatically revoked
- **AND** it SHALL advise reviewing the actual backend with its administrator

### Requirement: Grant confirmation uses verified destinations
The grant SHALL return a receipt only after identity, mode, capability, and enrollment verification succeeds.
The receipt SHALL identify the backend, route helper, and optional targeted anti-DPI helper.
The UI SHALL verify the receipt destinations with a fresh probe before reporting success.

#### Scenario: Targeted anti-DPI helper is not ready at the receipt path
- **WHEN** a grant receipt names an anti-DPI helper and the fresh probe reports another path or a non-ready verdict
- **THEN** the UI SHALL NOT report grant success

#### Scenario: Optional anti-DPI helper was not targeted
- **WHEN** the receipt omits the anti-DPI helper
- **THEN** an unrelated fixable or package-repair verdict SHALL NOT fail the grant
- **AND** a terminal anti-DPI verdict SHALL fail confirmation

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

### Requirement: Installers validate the complete payload before writing
The tarball installer SHALL determine the complete file set required by the explicitly selected release version before the first write under the installation prefix, and SHALL validate every required binary and share file in the extracted archive first. The set SHALL include `v2ray-rs-antidpi` only for a version that ships it; an older verified archive SHALL be accepted with its own set. A damaged current archive SHALL leave every existing file under the prefix unchanged.

#### Scenario: Older verified archive is accepted
- **WHEN** the user selects an older release version whose checksum verifies and whose archive carries the three pre-existing binaries
- **THEN** the installer SHALL validate that complete set and install it without requiring the newer helper

#### Scenario: Damaged archive changes nothing
- **WHEN** the archive is missing a required binary or share file
- **THEN** the installer SHALL fail before the first write and every file under the prefix SHALL remain unchanged

### Requirement: Arch removal distinguishes verified revocation
The Arch hook SHALL treat command success and the postcondition independently: a failed `setcap -r` or `chmod`, or a `getcap` invocation that cannot produce a verdict, SHALL be reported as unverified rather than as absence, for provisioning cleanup and for package removal.

#### Scenario: Unverifiable revocation is not success
- **WHEN** `getcap` fails to run or exits non-zero while checking a helper
- **THEN** the hook SHALL report the capability state as unresolved and fail the revocation step rather than claiming the privileges were removed

### Requirement: Staged catalogs are compiled from the current sources
The release staging step SHALL compile each shipped `.mo` from the tracked `.po` with `msgfmt` and SHALL fail when `msgfmt` is unavailable or a known translated key is missing from the staged catalog. It SHALL NOT copy a stale tracked `.mo`.

#### Scenario: Newly translated key reaches the staged payload
- **WHEN** a new user-facing string is added to the catalogs and a release tree is staged
- **THEN** `msgunfmt` on the staged `ru_RU` catalog SHALL contain that key

### Requirement: Debian asset sources resolve from the crate manifest
`[package.metadata.deb]` asset source paths that are not Cargo target artifacts SHALL be relative to the crate directory containing the manifest, and every one SHALL exist there. Cargo target artifacts SHALL keep their `target/release/...` form.

#### Scenario: Debian asset set resolves
- **WHEN** the Debian metadata is resolved from `crates/ui/Cargo.toml`
- **THEN** every static asset path SHALL resolve to an existing repository file and every listed binary SHALL resolve to the release artifact

### Requirement: CI checks all privileged test features and uninstallers report accurate cleanup
CI SHALL type-check `v2ray-rs-antidpi/privileged-tests` alongside `v2ray-rs-process` and `v2ray-rs-netctl`, `scripts/install.sh --uninstall` SHALL report relocated helper cleanup instead of `setcap -r` on already-deleted prefix binaries, and AppImage scripts and `docs/RELEASE.md` SHALL document `v2ray-rs-antidpi` parity.

#### Scenario: CI checks antidpi privileged tests
- **WHEN** the CI clippy/check workflow runs
- **THEN** `cargo check --workspace --all-targets` SHALL enable `v2ray-rs-antidpi/privileged-tests` alongside the other privileged test features

#### Scenario: Tarball uninstaller prints valid cleanup steps
- **WHEN** `scripts/install.sh --uninstall` removes binaries under `$PREFIX/bin`
- **THEN** its printed follow-up commands SHALL reference `/usr/local/lib/v2ray-rs/` relocated helpers rather than `setcap -r` on deleted `$PREFIX/bin` files
