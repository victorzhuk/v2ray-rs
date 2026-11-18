# Design

## Context

See `proposal.md` for motivation:
- `scripts/install-appimage.sh:27` sets `INSTALL_URL=".../install-appimage.sh"` for its own header/usage, then reuses `$INSTALL_URL` at line 509 inside the system-wide **tarball** `sudo` command, and expands `$USER` inside an unquoted heredoc at line 496 under `set -u` (`line 18`).
- `.github/workflows/release.yml:108-112` checks only `Cargo.toml` `version` against `$GITHUB_REF_NAME`, ignoring `pkg/archlinux/PKGBUILD` and `pkg/archlinux/bin/PKGBUILD`.
- `.github/workflows/release.yml:310-315` copies `pkg/archlinux/PKGBUILD` and `pkg/archlinux/bin/PKGBUILD` verbatim into `release-assets/`, shipping `sha256sums=('SKIP')` to users who download `PKGBUILD` / `PKGBUILD-bin` from GitHub Releases.

## Goals / Non-Goals

**Goals:**
- Ensure `install-appimage.sh` never directs administrators to pipe `install-appimage.sh` into `sudo sh`, and never aborts under `env -u USER` when printing TUN instructions.
- Fail `release.yml` fast in `tarball` (`Metadata` step) if either `PKGBUILD` `pkgver` drifts from `Cargo.toml`.
- Stamp exact 64-hex SHA-256 digests into `release-assets/PKGBUILD` and `release-assets/PKGBUILD-bin` before `softprops/action-gh-release` uploads them.

**Non-Goals:**
- Hardcoding release-specific SHA-256 sums into the tracked git copies of `pkg/archlinux/PKGBUILD` and `pkg/archlinux/bin/PKGBUILD` (which cannot know the tag tarball digest before tagging).

## Decisions

1. **Separate `TARBALL_INSTALL_URL` and safe user fallback in `scripts/install-appimage.sh`.**
   Define `TARBALL_INSTALL_URL="https://github.com/$REPO/releases/latest/download/install.sh"` (or derive from base URL) for the system-wide tarball hint, and compute `_user="${USER:-$(id -un 2>/dev/null || printf '<your-user>')}"` in `print_tun_instructions`. Extend `scripts/test-installers.sh` to run `install-appimage.sh` under `env -u USER` (without ` quiet` suppressing errors, and capturing output when `v2ray-rs` group is absent) and assert exit `0` plus `install.sh` (never `install-appimage.sh | sudo`) in stdout.

2. **Enforce 3-way version parity (`Cargo.toml`, `pkg/archlinux/PKGBUILD`, `pkg/archlinux/bin/PKGBUILD`) in `release.yml` (and CI).**
   In `release.yml`'s `Metadata` step (and optionally a fast check in `test-installers.sh` so PR CI catches version drift before tagging!), parse `pkgver` from both `PKGBUILD` files and fail if either differs from `Cargo.toml`'s `[workspace.package].version`.

3. **Stamp `sha256sums` in `release.yml` `Collect PKGBUILDs`.**
   - For `release-assets/PKGBUILD-bin`: read the already-verified 64-hex digest from `release-assets/v2ray-rs-x86_64-linux.tar.gz.sha256` and replace `sha256sums=('SKIP')` with `sha256sums=('<hex>')`, asserting `grep -Eq "^sha256sums=\('[0-9a-f]{64}'\)$"`.
   - For `release-assets/PKGBUILD`: fetch `https://github.com/${GITHUB_REPOSITORY}/archive/${GITHUB_REF_NAME}.tar.gz` (the exact git tag archive URL referenced by `source=`), compute `sha256sum`, substitute `sha256sums=('<hex>')`, and assert no `SKIP` remains.

## Risks / Trade-offs

- **[Risk] GitHub archive tarball fetch transient failure during `release` job** -> Use `curl --fail --location --retry 3 --max-time 120` (matching every other pinned download in `ci.yml` / `release.yml`) and verify non-empty 64-hex SHA-256 before substitution.
