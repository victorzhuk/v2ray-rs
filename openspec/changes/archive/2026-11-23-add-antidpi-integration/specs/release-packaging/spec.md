# Spec Delta: release-packaging

## RENAMED Requirements

- FROM: `### Requirement: Makefile covers all eight workspace crates`
- TO: `### Requirement: Makefile covers all nine workspace crates`

## MODIFIED Requirements

### Requirement: Makefile covers all nine workspace crates
The root `Makefile` SHALL define per-crate `build-*` and `test-*` targets for all 9 workspace crates (`core`, `subscription`, `process`, `netctl`, `run`, `tun-session`, `tray`, `ui`, `antidpi`) and SHALL pass `--workspace` on aggregate check, lint, fix, and documentation targets. The `dist` target SHALL build the anti-DPI helper statically against musl alongside the other helpers and stage it as `bin/v2ray-rs-antidpi` in the release tree.

#### Scenario: Aggregate check and per-crate targets cover every crate
- **WHEN** a developer runs `make check`, `make check-all`, or per-crate `make build-<crate>` / `make test-<crate>` targets
- **THEN** `v2ray-rs-netctl`, `v2ray-rs-run`, `v2ray-rs-antidpi`, and `v2ray-rs-tun-session` SHALL be checked, linted, and addressable alongside the other 5 crates

#### Scenario: dist tarball carries the anti-DPI helper
- **WHEN** `make dist` completes and the tarball is listed
- **THEN** the staged release tree SHALL contain `bin/v2ray-rs-antidpi` beside the other binaries, and the tarball installer and uninstaller SHALL treat it as one of the installed binaries

### Requirement: Distribution package metadata covers build tools, icons, and TUN helpers
Source and binary Linux package definitions SHALL declare required protobuf compiler build dependencies and valid asset paths: `pkg/archlinux/PKGBUILD` SHALL include `protobuf` in `makedepends`, SHALL build `v2ray-rs-ui`, `v2ray-rs-netctl`, `v2ray-rs-run`, and `v2ray-rs-antidpi`, and SHALL install each binary under `/usr/bin` with mode `755` (the install hook tightens the privileged helpers), and `[package.metadata.deb]` in `crates/ui/Cargo.toml` SHALL reference existing icon files, include `v2ray-rs-netctl`, `v2ray-rs-run`, and `v2ray-rs-antidpi` with mode `755`, and declare runtime shared-library dependencies.

#### Scenario: Arch source PKGBUILD declares protobuf makedepend
- **WHEN** `pkg/archlinux/PKGBUILD` is inspected or built in a clean Arch chroot
- **THEN** `makedepends` SHALL include `protobuf` alongside `rust` and `cargo` so `prost-build` finds `protoc`

#### Scenario: Arch source PKGBUILD builds and installs the anti-DPI helper
- **WHEN** `pkg/archlinux/PKGBUILD` is inspected
- **THEN** its `build()` SHALL include `-p v2ray-rs-antidpi` and `package()` SHALL install `v2ray-rs-antidpi` to `/usr/bin` with mode `755`

#### Scenario: Debian package metadata references existing assets and helpers
- **WHEN** `[package.metadata.deb]` in `crates/ui/Cargo.toml` is validated against repository paths and release binaries
- **THEN** every static icon/desktop asset path SHALL exist in the repository and `v2ray-rs`, `v2ray-rs-netctl`, `v2ray-rs-run`, and `v2ray-rs-antidpi` SHALL be mapped to `/usr/bin/` with mode `755`

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

## ADDED Requirements

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
