# Tasks

## 1. AppImage Installer Hint and Unset-USER Safety

- [x] 1.1 Update `print_tun_instructions` in `scripts/install-appimage.sh` to reference `install.sh` for the system-wide tarball command and resolve the target username safely when `USER` is unset under `set -u`; extend `scripts/test-installers.sh` to run `install-appimage.sh` with `env -u USER`, asserting exit `0` and verifying printed system-wide instructions reference `install.sh` rather than `install-appimage.sh`.

## 2. PKGBUILD Version Parity and Release Asset Checksums

- [x] 2.1 Add version-parity checks across `Cargo.toml`, `pkg/archlinux/PKGBUILD`, and `pkg/archlinux/bin/PKGBUILD` in `scripts/test-installers.sh` and `.github/workflows/release.yml` (`Metadata`), and verify `make test-installers` passes on matching versions and rejects a simulated mismatch.
- [x] 2.2 Update `Collect PKGBUILDs` in `.github/workflows/release.yml` to substitute verified 64-hex SHA-256 digests into `release-assets/PKGBUILD` and `release-assets/PKGBUILD-bin` and assert `SKIP` is absent, and update `docs/RELEASE.md` to document stamped release `PKGBUILD` digests.
