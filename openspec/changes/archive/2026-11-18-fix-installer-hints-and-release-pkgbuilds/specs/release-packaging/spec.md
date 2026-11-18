# Spec Delta

## ADDED Requirements

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
