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
| `PKGBUILD`, `PKGBUILD-bin` | `release` job | copies of the two Arch recipes with the release SHA-256 digests stamped into `sha256sums` — never `SKIP`; the tracked repo copies keep `SKIP` for the AUR `updpkgsums` flow |
| `v2ray-rs.install` | `release` job | Arch install scriptlet shared by `PKGBUILD` and `PKGBUILD-bin` |

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

The tag push runs the reusable quality workflow alongside packaging. Publication
requires quality checks, the tarball, the AppImage, and its smoke test to pass
for the tagged revision. The tag must match the workspace version, and so must both Arch `pkgver`
fields. The `Metadata` step runs inside the `tarball` job after its build
steps, so drift between `Cargo.toml`, `pkg/archlinux/PKGBUILD`, and
`pkg/archlinux/bin/PKGBUILD` blocks publication — everything downstream of the
tarball job: AppImage, release, AUR — not every build; the `quality` job and
the build steps that already ran are unaffected. Quality
includes required xray, v2ray, and sing-box checks, display-backed tests, and
isolated routing acceptance. Real Delay acceptance drives the production
measurement path against locally installed xray and v2ray serving a loopback
fixture, and checks that a reachable node alone receives a delay while probe
processes and temporary configs exit with the session. Missing required
backends, display, or routing prerequisites fail their jobs. Test commands have
deadlines and worker limits.

Backend and packaging downloads use fixed release asset IDs or a commit-pinned
source URL, and the pinned upstream downloads are verified against a SHA-256
digest before use. The release tarball is verified where its sidecar is
consumed: both installers check it before extraction, and the `release` job
checks it before stamping `PKGBUILD-bin`. The tagged source archive remains
the exception: it is hashed after download, and that digest is what the
`release` job stamps into `PKGBUILD`.
Updating a pin requires verifying the new upstream asset and its
digest together. A deleted asset fails the build; it never selects a replacement.
The selected linuxdeploy AppImage includes its AppImage output plugin and
appimagetool, so the same digest covers those bundled tools. The AppImage runtime
is separately pinned and verified, then supplied through `LDAI_RUNTIME_FILE`;
appimagetool must not fetch a replacement runtime during packaging.

The smoke test observes the application window under Xvfb, requests shutdown
with SIGTERM, and requires a successful exit within its deadline.

The AUR job runs after publication: `updpkgsums` for `v2ray-rs-bin` downloads the
release tarball, which does not exist until `release` has uploaded it.

`make dist` reproduces the tarball locally, but against this host's glibc — use
it to check the layout, not to check portability.

## Installer verification

Both installers refuse to modify the prefix unless the payload matches a
SHA-256 digest. They prefer the digest baked into the generated installer, then
the `.sha256` published beside the artifact. A missing `sha256sum`, a missing or
malformed sidecar, and a mismatched digest all exit unsuccessfully before
extraction, execution, or any prefix write; `--quiet` does not relax this. An
offline mirror must publish the matching `.sha256` beside the artifact, or the
installer must be regenerated with the artifact digest. `make test-installers`
exercises every failure mode against local fixtures, asserts that an existing
prefix is untouched, and checks that no downloaded payload runs. It also
verifies workspace/PKGBUILD version parity and the checksum stamping the
`release` job applies to the published `PKGBUILD` and `PKGBUILD-bin` assets.

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
  apt-get install -y -qq libgtk-4-1 libadwaita-1-0 xvfb xauth x11-utils &&
  ./v2ray-rs-x86_64.AppImage --help &&
  xvfb-run --auto-servernum timeout 35s bash <<'"'"'SMOKE'"'"'
  set -euo pipefail
  ./v2ray-rs-x86_64.AppImage --profile development &
  app_pid=$!
  trap "kill -KILL $app_pid 2>/dev/null || true" EXIT
  ready=0
  for attempt in $(seq 1 40); do
    kill -0 "$app_pid"
    if xwininfo -root -tree | grep -q "\"V2Ray Manager\""; then
      ready=1
      break
    fi
    sleep 0.25
  done
  if [ "$ready" -ne 1 ]; then
    echo "application window did not appear" >&2
    exit 1
  fi
  kill -TERM "$app_pid"
  for attempt in $(seq 1 40); do
    if ! kill -0 "$app_pid" 2>/dev/null; then
      wait "$app_pid"
      trap - EXIT
      exit 0
    fi
    sleep 0.25
  done
  echo "application did not shut down cleanly" >&2
  exit 1
SMOKE'
```

`ubuntu:24.04` carries GTK 4.14, below the 4.18 build floor, so nothing from
the build environment can mask a missing bundle entry. The two `apt-get`
packages are the supported minimum, not a workaround: a host carrying none of
the GTK stack is not a supported configuration, and the check would fail there.

What the check proves is narrow. `--help` exits before GTK initialisation, so
it proves the bundled loader resolves every library the bundle provides, and
nothing more. Only the `xvfb-run` step opens a window, exercising the pixbuf
loaders, GSettings schemas and the icon theme, which is where GTK4 bundling
actually breaks. The windowed step polls `xwininfo` for the `V2Ray Manager`
window and requires a clean exit after SIGTERM within its deadline, so a
bundle that never shows a window or fails to shut down fails the check; the
same observable gate is the `appimage-smoke` job in
`.github/workflows/release.yml`.

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

Provisioning starts from group-restricted helpers. The hook verifies that both
helpers exist, applies `root:v2ray-rs` ownership and mode 0750, verifies that
baseline, then grants `CAP_NET_ADMIN` to netctl and the setuid bit to run. It
verifies the final state — netctl 0750 with `cap_net_admin`, run 4750, both
owned by `root:v2ray-rs` — and on any failure reports the affected helper,
removes a partially applied capability or setuid bit, and leaves TUN disabled
until an operator repairs permissions. Removal strips the capability but keeps
the `v2ray-rs` group and `v2ray-rs-bypass` user, so a reinstall preserves their
UID, GID, and memberships. `make test-installers` exercises install, upgrade,
induced failure, removal, and reinstall against a temporary helper tree.

An AUR repository is created by the first push to its name, so no manual
registration is needed. The push only fails if someone else already owns
`v2ray-rs-bin`.
