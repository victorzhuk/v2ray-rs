# Releasing

## What a release produces

| Artifact | Built by | Notes |
| --- | --- | --- |
| `v2ray-rs-x86_64-linux.tar.gz` | `tarball` job | GUI + both helpers + desktop entry, icons, translations |
| `v2ray-rs-x86_64-linux.tar.gz.sha256` | `tarball` job | fallback digest when the installer's baked one does not apply |
| `install.sh` | `tarball` job | `scripts/install.sh` with the version and digest substituted |
| `v2ray-rs-x86_64.AppImage` | `appimage` job | bundles GTK 4.18, libadwaita, and `v2ray-rs-netctl` |
| `v2ray-rs-x86_64.AppImage.sha256` | `appimage` job | |
| `PKGBUILD`, `PKGBUILD-bin` | `release` job | copies of the two Arch recipes |

## Linkage

Only the two helpers are static. The GUI cannot be:
`relm4`'s `gnome_48` feature requires GTK 4.18 and libadwaita 1.7, and GTK4
`dlopen`s pixbuf loaders and GIO modules, which a `-static` musl binary cannot do.

| Binary | Target | Linkage |
| --- | --- | --- |
| `v2ray-rs` | `x86_64-unknown-linux-gnu` on `debian:trixie` | glibc 2.41, GTK dynamic |
| `v2ray-rs-netctl` | `x86_64-unknown-linux-musl` | static |
| `v2ray-rs-run` | `x86_64-unknown-linux-musl` | static |

`debian:trixie` is the oldest image carrying GTK 4.18.6, libadwaita 1.7, and
glibc 2.41 — exactly the floor `gnome_48` demands, so it maximises the set of
distributions the tarball runs on. The `tarball` job fails if `objdump` finds a
glibc symbol above `GLIBC_MAX`, so bumping the base image cannot silently raise
the floor. `container:` cannot read a workflow env value, so raising the floor
deliberately means editing three places in `.github/workflows/release.yml` — the
`tarball` and `appimage` job containers and `GLIBC_MAX` — and saying so in the
changelog.

Release builds use thin LTO, one codegen unit, and `strip = "debuginfo"`. The
last keeps `.symtab`, so a backtrace out of a shipped binary still names
functions.

## Cutting a release

1. Bump the version in **four** places:
   - `Cargo.toml` — `[workspace.package] version`
   - `pkg/archlinux/PKGBUILD` — `pkgver`
   - `pkg/archlinux/bin/PKGBUILD` — `pkgver`
   - `CHANGELOG.md` — new section plus the link refs at the bottom
2. `cargo check` to regenerate `Cargo.lock`.
3. Commit as `chore(release): bump version to X.Y.Z`.
4. `git tag vX.Y.Z && git push --tags`.

The tag push runs `helpers` → `tarball` → `appimage` → `release` → `aur`. The
AUR job runs last on purpose: `updpkgsums` for `v2ray-rs-bin` downloads the
release tarball, which does not exist until `release` has uploaded it.

`make dist` reproduces the tarball locally, but against this host's glibc — use
it to check the layout, not to check portability.

## Verifying a release

```bash
# Static helpers carry no runtime dependency
readelf -d target/x86_64-unknown-linux-musl/release/v2ray-rs-netctl | grep NEEDED   # no output

# The tarball installs and runs in a clean container
docker run --rm -it debian:trixie sh -c '
  apt-get update -qq && apt-get install -y -qq curl libgtk-4-1 libadwaita-1-0 &&
  curl -fsSL https://github.com/victorzhuk/v2ray-rs/releases/latest/download/install.sh | sh &&
  ~/.local/bin/v2ray-rs --help &&
  ~/.local/bin/v2ray-rs-netctl --version'

# The AppImage is self-contained on a host below the GTK floor
docker run --rm -v "$PWD:/w" -w /w -e APPIMAGE_EXTRACT_AND_RUN=1 \
  ubuntu:24.04 ./v2ray-rs-x86_64.AppImage --help
```

Re-running `install.sh` must be idempotent, and `install.sh --uninstall` must
leave no files behind under the prefix.

## AppImage payload

The AppImage bundles `v2ray-rs-netctl` but **not** `v2ray-rs-run`. The wrapper
drops to the `v2ray-rs-bypass` system user, which only a distribution package
creates, so a setuid-root copy shipped in an AppImage could not work. The
bundled `netctl` is inert where it sits — the squashfs is `nosuid` — and exists
only as the source the privileged grant copies to `/usr/local/lib/v2ray-rs/`.
See the TUN section of `docs/ARCHITECTURE.md`.

## The AUR packages

`v2ray-rs-bin` unpacks the release tarball; `v2ray-rs` compiles from the source
tarball. `v2ray-rs-bin` declares `provides`/`conflicts` on `v2ray-rs` — the
usual one-sided arrangement for a `-bin` sibling — so only one can be installed.
Both share `pkg/archlinux/v2ray-rs.install`, which creates the `v2ray-rs` group
and `v2ray-rs-bypass` user and applies the helper capabilities.

An AUR repository is created by the first push to its name, so no manual
registration is needed. The push only fails if someone else already owns
`v2ray-rs-bin`.
