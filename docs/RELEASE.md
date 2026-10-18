# Releasing

## What a release produces

| Artifact | Built by | Notes |
| --- | --- | --- |
| `v2ray-rs-x86_64-linux.tar.gz` | `tarball` job | GUI + both helpers + desktop entry, icons, translations |
| `v2ray-rs-x86_64-linux.tar.gz.sha256` | `tarball` job | fallback digest when the installer's baked one does not apply |
| `install.sh` | `tarball` job | `scripts/install.sh` with the version and digest substituted |
| `v2ray-rs-x86_64.AppImage` | `appimage` job | bundles GTK 4.18, libadwaita, and `v2ray-rs-netctl` |
| `v2ray-rs-x86_64.AppImage.sha256` | `appimage` job | |
| `install-appimage.sh` | `appimage` job | `scripts/install-appimage.sh` with the version substituted; installs the AppImage into a user prefix |
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
   CI's advisory-audit job reads `Cargo.lock` against the RustSec database and
   fails the build whenever a resolved dependency carries a vulnerability
   advisory. A deferred advisory needs a recorded exception naming an owner and
   a review date.

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

# The AppImage runs on a host below the GTK build floor, given the GTK runtime
docker run --rm -v "$PWD:/w" -w /w -e APPIMAGE_EXTRACT_AND_RUN=1 \
  ubuntu:24.04 sh -c '
  apt-get update -qq &&
  apt-get install -y -qq libgtk-4-1 libadwaita-1-0 xvfb xauth &&
  ./v2ray-rs-x86_64.AppImage --help &&
  status=0
  xvfb-run --auto-servernum timeout 20 ./v2ray-rs-x86_64.AppImage \
    --profile development || status=$?
  [ "$status" -eq 0 ] || [ "$status" -eq 124 ]'
```

`ubuntu:24.04` carries GTK 4.14, below the 4.18 build floor, so nothing from
the build environment can mask a missing bundle entry. The two `apt-get`
packages are the supported minimum, not a workaround: a host carrying none of
the GTK stack is not a supported configuration, and the check would fail there.

What the check proves is narrow. `--help` exits before GTK initialisation, so
it proves the bundled loader resolves every library the bundle provides, and
nothing more. Only the `xvfb-run` step opens a window, exercising the pixbuf
loaders, GSettings schemas and the icon theme, which is where GTK4 bundling
actually breaks. Exit 124 is `timeout` expiring on a healthy long-running
window; any other nonzero status fails the check.

Re-running either installer must be idempotent, and `--uninstall` on either one
must leave no files behind under the prefix.

## AppImage payload

The AppImage bundles `v2ray-rs-netctl` but **not** `v2ray-rs-run`. The wrapper
drops to the `v2ray-rs-bypass` system user, which only a distribution package
creates, so a setuid-root copy shipped in an AppImage could not work. The
bundled `netctl` is inert where it sits — the squashfs is `nosuid` — and exists
only as the source the privileged grant copies to `/usr/local/lib/v2ray-rs/`.
See the TUN section of `docs/ARCHITECTURE.md`.

The AppImage is not self-contained. It bundles GTK 4.18, libadwaita 1.7 and the
GTK and Pango module trees, so it runs on hosts below the GTK build floor, but
it relies on the host for the base font and rendering stack the AppImage
excludelist deliberately leaves out: GLib, Pango, fontconfig, harfbuzz, fribidi,
and the X11 and Wayland client libraries. Upstream excludes them because
bundling them is documented to break AppImages on some distributions — symbol
conflicts against a system X11, interaction with Mesa, and font rendering
stalls. The exclusion is a deliberate upstream decision, not an oversight.

The image also does not carry FUSE, a desktop session, or a proxy backend. It
needs FUSE to mount, with `APPIMAGE_EXTRACT_AND_RUN=1` as a slower fallback that
extracts and runs in place, an X11 or Wayland session to draw in, and one of
v2ray, xray or sing-box installed on the host.

`install-appimage.sh` installs the image into a user prefix. It extracts the
desktop entry and the icons out of the squashfs and rewrites `Exec=` to the
installed path, because the baked `Exec=v2ray-rs` only resolves when `$PREFIX/bin`
is on `$PATH` and it is not, from a desktop launcher. `$PREFIX/share/locale`
stays empty on purpose: the image carries its own gettext catalogs.

## The AUR packages

`v2ray-rs-bin` unpacks the release tarball; `v2ray-rs` compiles from the source
tarball. `v2ray-rs-bin` declares `provides`/`conflicts` on `v2ray-rs` — the
usual one-sided arrangement for a `-bin` sibling — so only one can be installed.
Both share `pkg/archlinux/v2ray-rs.install`, which creates the `v2ray-rs` group
and `v2ray-rs-bypass` user and applies the helper capabilities.

An AUR repository is created by the first push to its name, so no manual
registration is needed. The push only fails if someone else already owns
`v2ray-rs-bin`.
