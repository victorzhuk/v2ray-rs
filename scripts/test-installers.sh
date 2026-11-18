#!/usr/bin/env bash
# Exercises both release installers against local fixtures. Every failure mode
# must exit unsuccessfully, leave an existing prefix untouched, and avoid
# executing the downloaded payload.
set -uo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/v2ray-rs-installers.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT INT TERM

failures=0
note() { printf '\n== %s\n' "$*"; }
pass() { printf 'ok: %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; failures=$((failures + 1)); }

marker="$work/executed"
: >"$marker"

fixtures="$work/fixtures"
mkdir -p "$fixtures"

# --- tarball fixture -------------------------------------------------------

tar_root="$fixtures/v2ray-rs-1.2.3-x86_64-linux"
mkdir -p "$tar_root/bin" \
    "$tar_root/share/applications" \
    "$tar_root/share/icons/hicolor/scalable/apps" \
    "$tar_root/share/icons/hicolor/symbolic/apps" \
    "$tar_root/share/icons/hicolor/256x256/apps" \
    "$tar_root/share/locale/en_US/LC_MESSAGES" \
    "$tar_root/share/locale/ru_RU/LC_MESSAGES"
for bin in v2ray-rs v2ray-rs-netctl v2ray-rs-run; do
    printf '#!/bin/sh\nprintf "%%s\\n" "%s" >>"%s"\nexit 0\n' "$bin" "$marker" >"$tar_root/bin/$bin"
    chmod 0755 "$tar_root/bin/$bin"
done
for rel in \
    share/applications/com.github.v2ray-rs.desktop \
    share/icons/hicolor/scalable/apps/com.github.v2ray-rs.svg \
    share/icons/hicolor/symbolic/apps/com.github.v2ray-rs-symbolic.svg \
    share/icons/hicolor/256x256/apps/com.github.v2ray-rs.png \
    share/locale/en_US/LC_MESSAGES/v2ray-rs.mo \
    share/locale/ru_RU/LC_MESSAGES/v2ray-rs.mo; do
    printf 'fixture\n' >"$tar_root/$rel"
done
printf 'fixture license\n' >"$tar_root/LICENSE"

tarball="$fixtures/v2ray-rs-x86_64-linux.tar.gz"
tar -C "$fixtures" -czf "$tarball" "$(basename "$tar_root")"
tarball_sha=$(sha256sum -b "$tarball" | awk '{print $1}')
printf '%s  v2ray-rs-x86_64-linux.tar.gz\n' "$tarball_sha" >"$tarball.sha256"

# --- AppImage fixture ------------------------------------------------------

appimage="$fixtures/v2ray-rs-x86_64.AppImage"
cat >"$appimage" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >>"$marker"
case "\$1" in
  --appimage-extract)
    mkdir -p squashfs-root/usr/share/applications \\
        squashfs-root/usr/share/icons/hicolor/scalable/apps \\
        squashfs-root/usr/share/icons/hicolor/symbolic/apps \\
        squashfs-root/usr/share/icons/hicolor/256x256/apps
    printf 'Exec=v2ray-rs\nTryExec=v2ray-rs\nIcon=com.github.v2ray-rs\n' >squashfs-root/usr/share/applications/com.github.v2ray-rs.desktop
    printf 'fixture icon\n' >squashfs-root/usr/share/icons/hicolor/scalable/apps/com.github.v2ray-rs.svg
    printf 'fixture icon\n' >squashfs-root/usr/share/icons/hicolor/symbolic/apps/com.github.v2ray-rs-symbolic.svg
    printf 'fixture icon\n' >squashfs-root/usr/share/icons/hicolor/256x256/apps/com.github.v2ray-rs.png
    ;;
esac
exit 0
EOF
chmod 0755 "$appimage"
appimage_sha=$(sha256sum -b "$appimage" | awk '{print $1}')
printf '%s  v2ray-rs-x86_64.AppImage\n' "$appimage_sha" >"$appimage.sha256"

# --- fixture variants ------------------------------------------------------

no_sidecar="$work/no-sidecar"
malformed="$work/malformed"
mismatch="$work/mismatch"
for dir in "$no_sidecar" "$malformed" "$mismatch"; do
    mkdir -p "$dir"
    cp "$tarball" "$dir/v2ray-rs-x86_64-linux.tar.gz"
    cp "$appimage" "$dir/v2ray-rs-x86_64.AppImage"
done
: >"$no_sidecar/absent.marker"
printf 'not-a-digest\n' >"$malformed/v2ray-rs-x86_64-linux.tar.gz.sha256"
printf 'not-a-digest\n' >"$malformed/v2ray-rs-x86_64.AppImage.sha256"
zeroes=0000000000000000000000000000000000000000000000000000000000000000
printf '%s  v2ray-rs-x86_64-linux.tar.gz\n' "$zeroes" >"$mismatch/v2ray-rs-x86_64-linux.tar.gz.sha256"
printf '%s  v2ray-rs-x86_64.AppImage\n' "$zeroes" >"$mismatch/v2ray-rs-x86_64.AppImage.sha256"

# --- helpers ---------------------------------------------------------------

mkdir -p "$work/home"

seed_prefix() {
    mkdir -p "$1/bin"
    printf 'existing install\n' >"$1/bin/v2ray-rs"
}

assert_untouched() {
    local prefix=$1 name=$2
    [ "$(cat "$prefix/bin/v2ray-rs" 2>/dev/null)" = "existing install" ] ||
        fail "$name: the existing installation was modified"
    [ -z "$(find "$prefix" -mindepth 1 ! -path "$prefix/bin" ! -path "$prefix/bin/v2ray-rs" -print -quit)" ] ||
        fail "$name: files appeared under the prefix"
    [ ! -s "$marker" ] || fail "$name: the downloaded payload was executed"
}

run_installer() {
    local script=$1 prefix=$2 url=$3
    shift 3
    : >"$marker"
    env HOME="$work/home" V2RAY_RS_INSTALL_DIR="$prefix" \
        V2RAY_RS_DOWNLOAD_URL="$url" V2RAY_RS_NO_MODIFY_PATH=1 \
        sh "$script" "$@"
}

expect_failure() {
    local name=$1 expected=$2 script=$3 prefix=$4 url=$5
    shift 5
    local log="$work/$name.log"

    seed_prefix "$prefix"
    if run_installer "$script" "$prefix" "$url" "$@" >"$log" 2>&1; then
        fail "$name: installer exited successfully"
        return
    fi
    grep -q "$expected" "$log" || fail "$name: missing '$expected' in the output"
    assert_untouched "$prefix" "$name"
    pass "$name"
}

expect_success() {
    local name=$1 script=$2 prefix=$3 url=$4
    shift 4
    local log="$work/$name.log"

    if ! run_installer "$script" "$prefix" "$url" "$@" >"$log" 2>&1; then
        fail "$name: installer exited unsuccessfully: $(tail -n 3 "$log" | tr '\n' ' ')"
        return
    fi
    pass "$name"
}

make_minbin() {
    local dir=$1 bin
    mkdir -p "$dir"
    for bin in sh uname mktemp install rm sed awk cat grep tput curl tar gzip \
        dirname id mkdir chmod cp mv ln env; do
        if path=$(command -v "$bin" 2>/dev/null); then
            ln -s "$path" "$dir/$bin"
        fi
    done
}

minbin="$work/minbin"
make_minbin "$minbin"

# A curl stand-in that serves the fixtures for any release URL, so the pinned
# installer (which cannot use V2RAY_RS_DOWNLOAD_URL without losing its digest)
# still performs a real end-to-end install.
fakebin="$work/fakebin"
mkdir -p "$fakebin"
cat >"$fakebin/curl" <<'EOF'
#!/bin/sh
url=""
out=""
while [ $# -gt 0 ]; do
    case "$1" in
        -o) out=$2; shift 2 ;;
        --) shift ;;
        -*) shift ;;
        *) url=$1; shift ;;
    esac
done
[ -n "$url" ] && [ -n "$out" ] || exit 2
printf '%s\n' "$url" >>"$CURL_LOG"
case "$url" in
    *v2ray-rs-x86_64-linux.tar.gz.sha256) cp "$FIXTURE_TARBALL_SHA" "$out" ;;
    *v2ray-rs-x86_64-linux.tar.gz) cp "$FIXTURE_TARBALL" "$out" ;;
    *v2ray-rs-x86_64.AppImage.sha256) cp "$FIXTURE_APPIMAGE_SHA" "$out" ;;
    *v2ray-rs-x86_64.AppImage) cp "$FIXTURE_APPIMAGE" "$out" ;;
    *) exit 22 ;;
esac
EOF
chmod 0755 "$fakebin/curl"

# --- tarball installer -----------------------------------------------------

note "tarball installer"
tar_installer="$repo/scripts/install.sh"

expect_failure "tar-missing-sidecar" "no checksum available" \
    "$tar_installer" "$work/p-tar-missing" "file://$no_sidecar/v2ray-rs-x86_64-linux.tar.gz"
expect_failure "tar-malformed-digest" "no checksum available" \
    "$tar_installer" "$work/p-tar-malformed" "file://$malformed/v2ray-rs-x86_64-linux.tar.gz"
expect_failure "tar-digest-mismatch" "checksum mismatch" \
    "$tar_installer" "$work/p-tar-mismatch" "file://$mismatch/v2ray-rs-x86_64-linux.tar.gz"
expect_failure "tar-digest-mismatch-quiet" "checksum mismatch" \
    "$tar_installer" "$work/p-tar-quiet" "file://$mismatch/v2ray-rs-x86_64-linux.tar.gz" -q

seed_prefix "$work/p-tar-nosha"
: >"$marker"
if env HOME="$work/home" V2RAY_RS_INSTALL_DIR="$work/p-tar-nosha" \
    V2RAY_RS_DOWNLOAD_URL="file://$fixtures/v2ray-rs-x86_64-linux.tar.gz" \
    V2RAY_RS_NO_MODIFY_PATH=1 PATH="$minbin" \
    sh "$tar_installer" >"$work/tar-nosha.log" 2>&1; then
    fail "tar-missing-verifier: installer exited successfully"
else
    grep -q "sha256sum not found" "$work/tar-nosha.log" ||
        fail "tar-missing-verifier: missing verifier was not reported"
    assert_untouched "$work/p-tar-nosha" "tar-missing-verifier"
    pass "tar-missing-verifier"
fi

expect_success "tar-sidecar-install" "$tar_installer" "$work/p-tar-sidecar" \
    "file://$fixtures/v2ray-rs-x86_64-linux.tar.gz"
[ -x "$work/p-tar-sidecar/bin/v2ray-rs" ] || fail "tar-sidecar-install: binary missing"

pinned_installer="$work/install-pinned.sh"
bash "$repo/scripts/gen-installer.sh" 1.2.3 "$tarball" "$pinned_installer" >/dev/null ||
    fail "gen-installer failed"

: >"$work/curl-log"
: >"$marker"
if env -u V2RAY_RS_VERSION -u V2RAY_RS_DOWNLOAD_URL \
    HOME="$work/home" V2RAY_RS_INSTALL_DIR="$work/p-tar-pinned" \
    V2RAY_RS_NO_MODIFY_PATH=1 PATH="$fakebin:$PATH" \
    CURL_LOG="$work/curl-log" FIXTURE_TARBALL="$tarball" \
    FIXTURE_TARBALL_SHA="$tarball.sha256" \
    sh "$pinned_installer" -v >"$work/tar-pinned.log" 2>&1; then
    [ -x "$work/p-tar-pinned/bin/v2ray-rs" ] || fail "tar-pinned-install: binary missing"
    grep -q "pinned in this installer" "$work/tar-pinned.log" ||
        fail "tar-pinned-install: baked digest was not used"
    grep -q "v1.2.3" "$work/curl-log" || fail "tar-pinned-install: wrong release URL"
    grep -q '\.sha256' "$work/curl-log" && fail "tar-pinned-install: sidecar was fetched"
    pass "tar-pinned-install"
else
    fail "tar-pinned-install: installer failed: $(tail -n 3 "$work/tar-pinned.log" | tr '\n' ' ')"
fi

: >"$work/curl-log"
if env V2RAY_RS_VERSION=2.0.0 HOME="$work/home" V2RAY_RS_INSTALL_DIR="$work/p-tar-alt" \
    V2RAY_RS_NO_MODIFY_PATH=1 PATH="$fakebin:$PATH" \
    CURL_LOG="$work/curl-log" FIXTURE_TARBALL="$tarball" \
    FIXTURE_TARBALL_SHA="$tarball.sha256" \
    sh "$pinned_installer" >"$work/tar-alt.log" 2>&1; then
    [ -x "$work/p-tar-alt/bin/v2ray-rs" ] || fail "tar-alternate-version: binary missing"
    grep -q "v2.0.0" "$work/curl-log" || fail "tar-alternate-version: wrong release URL"
    grep -q '\.sha256' "$work/curl-log" || fail "tar-alternate-version: sidecar was not fetched"
    pass "tar-alternate-version"
else
    fail "tar-alternate-version: installer failed: $(tail -n 3 "$work/tar-alt.log" | tr '\n' ' ')"
fi

# --- AppImage installer ----------------------------------------------------

note "AppImage installer"
appimage_installer="$repo/scripts/install-appimage.sh"

expect_failure "appimage-missing-sidecar" "no checksum available" \
    "$appimage_installer" "$work/p-app-missing" "file://$no_sidecar/v2ray-rs-x86_64.AppImage"
expect_failure "appimage-malformed-digest" "no checksum available" \
    "$appimage_installer" "$work/p-app-malformed" "file://$malformed/v2ray-rs-x86_64.AppImage"
expect_failure "appimage-digest-mismatch" "checksum mismatch" \
    "$appimage_installer" "$work/p-app-mismatch" "file://$mismatch/v2ray-rs-x86_64.AppImage"
expect_failure "appimage-digest-mismatch-quiet" "checksum mismatch" \
    "$appimage_installer" "$work/p-app-quiet" "file://$mismatch/v2ray-rs-x86_64.AppImage" -q

seed_prefix "$work/p-app-nosha"
: >"$marker"
if env HOME="$work/home" V2RAY_RS_INSTALL_DIR="$work/p-app-nosha" \
    V2RAY_RS_DOWNLOAD_URL="file://$fixtures/v2ray-rs-x86_64.AppImage" \
    V2RAY_RS_NO_MODIFY_PATH=1 PATH="$minbin" \
    sh "$appimage_installer" >"$work/app-nosha.log" 2>&1; then
    fail "appimage-missing-verifier: installer exited successfully"
else
    grep -q "sha256sum not found" "$work/app-nosha.log" ||
        fail "appimage-missing-verifier: missing verifier was not reported"
    assert_untouched "$work/p-app-nosha" "appimage-missing-verifier"
    pass "appimage-missing-verifier"
fi

expect_success "appimage-sidecar-install" "$appimage_installer" "$work/p-app-sidecar" \
    "file://$fixtures/v2ray-rs-x86_64.AppImage"
[ -x "$work/p-app-sidecar/bin/v2ray-rs.AppImage" ] ||
    fail "appimage-sidecar-install: AppImage missing"
grep -q "Exec=$work/p-app-sidecar/bin/v2ray-rs.AppImage" \
    "$work/p-app-sidecar/share/applications/com.github.v2ray-rs.desktop" ||
    fail "appimage-sidecar-install: desktop Exec was not rewritten"

pinned_app="$work/install-appimage-pinned.sh"
bash "$repo/scripts/gen-installer-appimage.sh" 1.2.3 "$appimage" "$pinned_app" >/dev/null ||
    fail "gen-installer-appimage failed"

: >"$work/curl-log"
if env -u V2RAY_RS_VERSION -u V2RAY_RS_DOWNLOAD_URL \
    HOME="$work/home" V2RAY_RS_INSTALL_DIR="$work/p-app-pinned" \
    V2RAY_RS_NO_MODIFY_PATH=1 PATH="$fakebin:$PATH" \
    CURL_LOG="$work/curl-log" FIXTURE_APPIMAGE="$appimage" \
    FIXTURE_APPIMAGE_SHA="$appimage.sha256" \
    sh "$pinned_app" -v >"$work/app-pinned.log" 2>&1; then
    [ -x "$work/p-app-pinned/bin/v2ray-rs.AppImage" ] ||
        fail "appimage-pinned-install: AppImage missing"
    grep -q "pinned in this installer" "$work/app-pinned.log" ||
        fail "appimage-pinned-install: baked digest was not used"
    grep -q "v1.2.3" "$work/curl-log" || fail "appimage-pinned-install: wrong release URL"
    pass "appimage-pinned-install"
else
    fail "appimage-pinned-install: installer failed: $(tail -n 3 "$work/app-pinned.log" | tr '\n' ' ')"
fi

: >"$work/curl-log"
if env V2RAY_RS_VERSION=2.0.0 HOME="$work/home" V2RAY_RS_INSTALL_DIR="$work/p-app-alt" \
    V2RAY_RS_NO_MODIFY_PATH=1 PATH="$fakebin:$PATH" \
    CURL_LOG="$work/curl-log" FIXTURE_APPIMAGE="$appimage" \
    FIXTURE_APPIMAGE_SHA="$appimage.sha256" \
    sh "$pinned_app" >"$work/app-alt.log" 2>&1; then
    [ -x "$work/p-app-alt/bin/v2ray-rs.AppImage" ] ||
        fail "appimage-alternate-version: AppImage missing"
    grep -q "v2.0.0" "$work/curl-log" || fail "appimage-alternate-version: wrong release URL"
    grep -q '\.sha256' "$work/curl-log" ||
        fail "appimage-alternate-version: sidecar was not fetched"
    pass "appimage-alternate-version"
else
    fail "appimage-alternate-version: installer failed: $(tail -n 3 "$work/app-alt.log" | tr '\n' ' ')"
fi

# A stub getent fails the group probe, so the TUN instructions branch runs
# deterministically even on hosts where the v2ray-rs group already exists.
tunbin="$work/tunbin"
mkdir -p "$tunbin"
printf '#!/bin/sh\nexit 1\n' >"$tunbin/getent"
chmod 0755 "$tunbin/getent"

seed_prefix "$work/p-app-tun"
: >"$marker"
if env -u USER HOME="$work/home" V2RAY_RS_INSTALL_DIR="$work/p-app-tun" \
    V2RAY_RS_DOWNLOAD_URL="file://$fixtures/v2ray-rs-x86_64.AppImage" \
    V2RAY_RS_NO_MODIFY_PATH=1 PATH="$tunbin:$PATH" \
    sh "$appimage_installer" >"$work/app-tun.log" 2>&1; then
    [ -x "$work/p-app-tun/bin/v2ray-rs.AppImage" ] ||
        fail "appimage-tun-unset-user: AppImage missing"
    grep -q 'install.sh | sudo' "$work/app-tun.log" ||
        fail "appimage-tun-unset-user: the tarball hint does not name install.sh"
    if grep -q 'install-appimage.sh | sudo' "$work/app-tun.log"; then
        fail "appimage-tun-unset-user: the tarball hint pipes install-appimage.sh into sudo"
    fi
    grep -q 'usermod -aG' "$work/app-tun.log" ||
        fail "appimage-tun-unset-user: usermod guidance missing"
    pass "appimage-tun-unset-user"
else
    fail "appimage-tun-unset-user: installer failed: $(tail -n 3 "$work/app-tun.log" | tr '\n' ' ')"
fi

# --- PKGBUILD parity and release stamping -----------------------------------

note "PKGBUILD version parity"
if sh "$repo/scripts/check-pkgbuild-version.sh" "$repo"; then
    pass "pkgbuild-version-parity"
else
    fail "pkgbuild-version-parity: workspace and PKGBUILD versions disagree"
fi

parity="$work/parity"
mkdir -p "$parity/pkg/archlinux/bin"
cp "$repo/Cargo.toml" "$parity/Cargo.toml"
cp "$repo/pkg/archlinux/PKGBUILD" "$parity/pkg/archlinux/PKGBUILD"
sed 's/^pkgver=.*/pkgver=9.9.9/' "$repo/pkg/archlinux/bin/PKGBUILD" \
    >"$parity/pkg/archlinux/bin/PKGBUILD"
if sh "$repo/scripts/check-pkgbuild-version.sh" "$parity" \
    >"$work/parity-drift.log" 2>&1; then
    fail "pkgbuild-version-drift: a mismatched pkgver was accepted"
else
    grep -q "bin/PKGBUILD" "$work/parity-drift.log" ||
        fail "pkgbuild-version-drift: the error does not name the mismatching file"
    pass "pkgbuild-version-drift"
fi

# A single-quoted pkgver and a CRLF pkgver must still pass parity.
ws_version=$(awk '
    /^\[.*\]/ { in_ws = ($0 == "[workspace.package]"); next }
    in_ws && /^[ \t]*version[ \t]*=[ \t]*"/ {
        sub(/^[ \t]*version[ \t]*=[ \t]*"/, "")
        sub(/".*/, "")
        print
        exit
    }
' "$repo/Cargo.toml")
parity_quoted="$work/parity-quoted"
mkdir -p "$parity_quoted/pkg/archlinux/bin"
cp "$repo/Cargo.toml" "$parity_quoted/Cargo.toml"
sed "s/^pkgver=.*/pkgver='$ws_version'/" "$repo/pkg/archlinux/PKGBUILD" \
    >"$parity_quoted/pkg/archlinux/PKGBUILD"
sed "s/^pkgver=.*/pkgver=$ws_version/" "$repo/pkg/archlinux/bin/PKGBUILD" |
    while IFS= read -r line; do printf '%s\r\n' "$line"; done \
    >"$parity_quoted/pkg/archlinux/bin/PKGBUILD"
if sh "$repo/scripts/check-pkgbuild-version.sh" "$parity_quoted"; then
    pass "pkgbuild-version-quoted-crlf"
else
    fail "pkgbuild-version-quoted-crlf: a single-quoted or CRLF pkgver was rejected"
fi

printed=$(sh "$repo/scripts/check-pkgbuild-version.sh" --print-version "$repo")
if [ "$printed" = "$ws_version" ]; then
    pass "pkgbuild-print-version"
else
    fail "pkgbuild-print-version: --print-version emitted '${printed:-<nothing>}', expected '$ws_version'"
fi

parity_padded="$work/parity-padded"
mkdir -p "$parity_padded/pkg/archlinux/bin"
sed 's/^\[workspace\.package\]/  [ workspace.package ]  /' "$repo/Cargo.toml" \
    >"$parity_padded/Cargo.toml"
cp "$repo/pkg/archlinux/PKGBUILD" "$parity_padded/pkg/archlinux/PKGBUILD"
cp "$repo/pkg/archlinux/bin/PKGBUILD" "$parity_padded/pkg/archlinux/bin/PKGBUILD"
if sh "$repo/scripts/check-pkgbuild-version.sh" "$parity_padded"; then
    pass "pkgbuild-version-padded-header"
else
    fail "pkgbuild-version-padded-header: a whitespace-padded table header was rejected"
fi

note "release PKGBUILD checksum stamping"
src_archive="$work/src-archive-1.2.3.tar.gz"
printf 'fixture source archive\n' >"$src_archive"
src_sha=$(sha256sum -b "$src_archive" | awk '{print $1}')

stamp_assets="$work/stamp-assets"
mkdir -p "$stamp_assets"
cp "$repo/pkg/archlinux/PKGBUILD" "$stamp_assets/PKGBUILD"
cp "$repo/pkg/archlinux/bin/PKGBUILD" "$stamp_assets/PKGBUILD-bin"
printf '%s  v2ray-rs-x86_64-linux.tar.gz\n' "$tarball_sha" \
    >"$stamp_assets/v2ray-rs-x86_64-linux.tar.gz.sha256"
cp "$tarball" "$stamp_assets/v2ray-rs-x86_64-linux.tar.gz"

stamp_bad="$work/stamp-bad"
mkdir -p "$stamp_bad"
cp "$repo/pkg/archlinux/PKGBUILD" "$stamp_bad/PKGBUILD"
cp "$repo/pkg/archlinux/bin/PKGBUILD" "$stamp_bad/PKGBUILD-bin"
printf 'not-a-digest\n' >"$stamp_bad/v2ray-rs-x86_64-linux.tar.gz.sha256"

stampbin="$work/stampbin"
mkdir -p "$stampbin"
cat >"$stampbin/curl" <<EOF
#!/bin/sh
url=""
out=""
while [ \$# -gt 0 ]; do
    case "\$1" in
        -o) out=\$2; shift 2 ;;
        -*) shift ;;
        *) url=\$1; shift ;;
    esac
done
[ -n "\$url" ] && [ -n "\$out" ] || exit 2
case "\$url" in
    *archive/v1.2.3.tar.gz) cp "$src_archive" "\$out" ;;
    *) exit 22 ;;
esac
EOF
chmod 0755 "$stampbin/curl"

if env GITHUB_REPOSITORY=victorzhuk/v2ray-rs GITHUB_REF_NAME=v1.2.3 \
    PATH="$stampbin:$PATH" TMPDIR="$work" \
    sh "$repo/scripts/stamp-release-pkgbuilds.sh" "$stamp_assets" \
    >"$work/stamp.log" 2>&1; then
    grep -q "sha256sums=('$src_sha')" "$stamp_assets/PKGBUILD" ||
        fail "pkgbuild-stamp: the source digest is missing"
    grep -q "sha256sums=('$tarball_sha')" "$stamp_assets/PKGBUILD-bin" ||
        fail "pkgbuild-stamp: the tarball digest is missing"
    if grep -q 'SKIP' "$stamp_assets/PKGBUILD" "$stamp_assets/PKGBUILD-bin"; then
        fail "pkgbuild-stamp: SKIP remains in a stamped PKGBUILD"
    fi
    pass "pkgbuild-stamp"
else
    fail "pkgbuild-stamp: stamping failed: $(tail -n 3 "$work/stamp.log" | tr '\n' ' ')"
fi

if env GITHUB_REPOSITORY=victorzhuk/v2ray-rs GITHUB_REF_NAME=v1.2.3 \
    PATH="$stampbin:$PATH" TMPDIR="$work" \
    sh "$repo/scripts/stamp-release-pkgbuilds.sh" "$stamp_bad" \
    >"$work/stamp-bad.log" 2>&1; then
    fail "pkgbuild-stamp-malformed: a malformed sidecar was accepted"
else
    grep -q "not 64 lowercase hex characters" "$work/stamp-bad.log" ||
        fail "pkgbuild-stamp-malformed: the failure reason is missing"
    grep -q 'SKIP' "$stamp_bad/PKGBUILD" "$stamp_bad/PKGBUILD-bin" ||
        fail "pkgbuild-stamp-malformed: files were modified before validation"
    pass "pkgbuild-stamp-malformed"
fi

stamp_stale="$work/stamp-stale"
mkdir -p "$stamp_stale"
cp "$repo/pkg/archlinux/PKGBUILD" "$stamp_stale/PKGBUILD"
sed "s/^sha256sums=('SKIP')/sha256sums=('$zeroes')/" \
    "$repo/pkg/archlinux/bin/PKGBUILD" >"$stamp_stale/PKGBUILD-bin"
printf '%s  v2ray-rs-x86_64-linux.tar.gz\n' "$tarball_sha" \
    >"$stamp_stale/v2ray-rs-x86_64-linux.tar.gz.sha256"
cp "$tarball" "$stamp_stale/v2ray-rs-x86_64-linux.tar.gz"
if env GITHUB_REPOSITORY=victorzhuk/v2ray-rs GITHUB_REF_NAME=v1.2.3 \
    PATH="$stampbin:$PATH" TMPDIR="$work" \
    sh "$repo/scripts/stamp-release-pkgbuilds.sh" "$stamp_stale" \
    >"$work/stamp-stale.log" 2>&1; then
    fail "pkgbuild-stamp-stale: a recipe without the SKIP placeholder was accepted"
else
    grep -q "PKGBUILD-bin" "$work/stamp-stale.log" ||
        fail "pkgbuild-stamp-stale: the failure does not name the stale file"
    grep -q 'SKIP' "$stamp_stale/PKGBUILD" ||
        fail "pkgbuild-stamp-stale: files were modified before validation"
    pass "pkgbuild-stamp-stale"
fi

stamp_tar_mismatch="$work/stamp-tar-mismatch"
mkdir -p "$stamp_tar_mismatch"
cp "$repo/pkg/archlinux/PKGBUILD" "$stamp_tar_mismatch/PKGBUILD"
cp "$repo/pkg/archlinux/bin/PKGBUILD" "$stamp_tar_mismatch/PKGBUILD-bin"
printf '%s  v2ray-rs-x86_64-linux.tar.gz\n' "$zeroes" \
    >"$stamp_tar_mismatch/v2ray-rs-x86_64-linux.tar.gz.sha256"
cp "$tarball" "$stamp_tar_mismatch/v2ray-rs-x86_64-linux.tar.gz"
if env GITHUB_REPOSITORY=victorzhuk/v2ray-rs GITHUB_REF_NAME=v1.2.3 \
    PATH="$stampbin:$PATH" TMPDIR="$work" \
    sh "$repo/scripts/stamp-release-pkgbuilds.sh" "$stamp_tar_mismatch" \
    >"$work/stamp-tar-mismatch.log" 2>&1; then
    fail "pkgbuild-stamp-tarball-mismatch: a mismatched staged tarball was accepted"
else
    grep -q "does not match" "$work/stamp-tar-mismatch.log" ||
        fail "pkgbuild-stamp-tarball-mismatch: the mismatch was not reported"
    grep -q 'SKIP' "$stamp_tar_mismatch/PKGBUILD" ||
        fail "pkgbuild-stamp-tarball-mismatch: PKGBUILD was modified before validation"
    grep -q 'SKIP' "$stamp_tar_mismatch/PKGBUILD-bin" ||
        fail "pkgbuild-stamp-tarball-mismatch: PKGBUILD-bin was modified before validation"
    pass "pkgbuild-stamp-tarball-mismatch"
fi

# A recipe carrying a second sha256sums= assignment must be rejected: the later
# assignment wins in makepkg, so one stamped line proves nothing.
stamp_dup="$work/stamp-dup"
mkdir -p "$stamp_dup"
cp "$repo/pkg/archlinux/PKGBUILD" "$stamp_dup/PKGBUILD"
{
    cat "$repo/pkg/archlinux/bin/PKGBUILD"
    printf "\nsha256sums=('%s')\n" "$tarball_sha"
} >"$stamp_dup/PKGBUILD-bin"
printf '%s  v2ray-rs-x86_64-linux.tar.gz\n' "$tarball_sha" \
    >"$stamp_dup/v2ray-rs-x86_64-linux.tar.gz.sha256"
cp "$tarball" "$stamp_dup/v2ray-rs-x86_64-linux.tar.gz"
if env GITHUB_REPOSITORY=victorzhuk/v2ray-rs GITHUB_REF_NAME=v1.2.3 \
    PATH="$stampbin:$PATH" TMPDIR="$work" \
    sh "$repo/scripts/stamp-release-pkgbuilds.sh" "$stamp_dup" \
    >"$work/stamp-dup.log" 2>&1; then
    fail "pkgbuild-stamp-duplicate-sums: a recipe with two sha256sums= assignments was accepted"
else
    grep -q "exactly one sha256sums=" "$work/stamp-dup.log" ||
        fail "pkgbuild-stamp-duplicate-sums: the duplicate assignment was not reported"
    grep -q 'SKIP' "$stamp_dup/PKGBUILD" ||
        fail "pkgbuild-stamp-duplicate-sums: PKGBUILD was modified before validation"
    grep -q "sha256sums=('SKIP')" "$stamp_dup/PKGBUILD-bin" ||
        fail "pkgbuild-stamp-duplicate-sums: PKGBUILD-bin was modified before validation"
    pass "pkgbuild-stamp-duplicate-sums"
fi

# A sidecar whose filename names another file must not make the staged tarball
# pass: the tarball is hashed directly and compared with the sidecar digest.
stamp_decoy="$work/stamp-decoy"
mkdir -p "$stamp_decoy"
cp "$repo/pkg/archlinux/PKGBUILD" "$stamp_decoy/PKGBUILD"
cp "$repo/pkg/archlinux/bin/PKGBUILD" "$stamp_decoy/PKGBUILD-bin"
printf 'decoy payload\n' >"$stamp_decoy/decoy.txt"
decoy_sha=$(sha256sum "$stamp_decoy/decoy.txt" | awk '{print $1}')
printf '%s  decoy.txt\n' "$decoy_sha" \
    >"$stamp_decoy/v2ray-rs-x86_64-linux.tar.gz.sha256"
cp "$tarball" "$stamp_decoy/v2ray-rs-x86_64-linux.tar.gz"
if env GITHUB_REPOSITORY=victorzhuk/v2ray-rs GITHUB_REF_NAME=v1.2.3 \
    PATH="$stampbin:$PATH" TMPDIR="$work" \
    sh "$repo/scripts/stamp-release-pkgbuilds.sh" "$stamp_decoy" \
    >"$work/stamp-decoy.log" 2>&1; then
    fail "pkgbuild-stamp-sidecar-decoy: a sidecar naming another file was accepted"
else
    grep -q "does not match" "$work/stamp-decoy.log" ||
        fail "pkgbuild-stamp-sidecar-decoy: the mismatch was not reported"
    grep -q 'SKIP' "$stamp_decoy/PKGBUILD" "$stamp_decoy/PKGBUILD-bin" ||
        fail "pkgbuild-stamp-sidecar-decoy: files were modified before validation"
    if grep -q "sha256sums=('$decoy_sha')" "$stamp_decoy/PKGBUILD-bin"; then
        fail "pkgbuild-stamp-sidecar-decoy: the decoy digest was stamped"
    fi
    pass "pkgbuild-stamp-sidecar-decoy"
fi

# --- result ----------------------------------------------------------------

if [ "$failures" -ne 0 ]; then
    printf '\n%d installer check(s) failed\n' "$failures" >&2
    exit 1
fi
printf '\nall installer checks passed\n'
