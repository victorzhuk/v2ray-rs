# Proposal

## Why

`scripts/install-appimage.sh` instructs users to run `curl ... $INSTALL_URL | sudo env V2RAY_RS_INSTALL_DIR=/usr/local sh` for the "system-wide tarball install", where `$INSTALL_URL` actually points to `install-appimage.sh` itself (running the AppImage as root instead of installing the tarball), and expands unguarded `$USER` under `set -u`. In `.github/workflows/release.yml`, Arch `PKGBUILD` `pkgver` fields are never checked against the release tag and GitHub Release `PKGBUILD` / `PKGBUILD-bin` assets are published with `sha256sums=('SKIP')`.

## What Changes

- Point the system-wide tarball install hint in `scripts/install-appimage.sh` at `install.sh` instead of `install-appimage.sh`, and guard `$USER` expansion under `set -u` when `USER` is unset in minimal shells or containers.
- Verify in `.github/workflows/release.yml` that `pkgver` in both `pkg/archlinux/PKGBUILD` and `pkg/archlinux/bin/PKGBUILD` matches the workspace `Cargo.toml` version (and git tag on tag pushes) before building/publishing release assets.
- Populate real SHA-256 checksums into the `release-assets/PKGBUILD` and `release-assets/PKGBUILD-bin` copies uploaded to GitHub Releases so `makepkg` verifies downloaded archives instead of skipping integrity checks.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `release-packaging`: Fix AppImage TUN guidance URL and unset-`USER` handling, enforce `PKGBUILD` version parity in CI, and stamp verified SHA-256 digests into release `PKGBUILD` assets.

## Impact

`scripts/install-appimage.sh`, `scripts/test-installers.sh`, `.github/workflows/release.yml`, and `docs/RELEASE.md`. AUR publishing (`aur-publish.yml`) continues using `updpkgsums: true` on repository `PKGBUILD`s without conflict.
