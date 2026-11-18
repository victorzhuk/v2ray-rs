#!/bin/sh
# Stamps verified SHA-256 digests into the staged release-asset PKGBUILDs.
#
# Usage: stamp-release-pkgbuilds.sh <assets-dir>
#
# <assets-dir> holds the staged release assets: PKGBUILD, PKGBUILD-bin, the
# v2ray-rs-x86_64-linux.tar.gz tarball, and its .sha256 sidecar. The tarball is
# verified against the sidecar before any digest is stamped. GITHUB_REPOSITORY and
# GITHUB_REF_NAME identify the tagged source archive whose digest replaces SKIP
# in PKGBUILD; the PKGBUILD-bin digest comes from the sidecar the tarball job
# wrote. The tracked repository PKGBUILDs keep their updpkgsums flow untouched.
set -eu

die() {
    printf 'stamp-release-pkgbuilds: %s\n' "$*" >&2
    exit 1
}

cleanup_files=
trap 'for f in $cleanup_files; do rm -f "$f"; done' EXIT INT TERM HUP

assert_digest() {
    printf '%s\n' "$1" | grep -Eq '^[0-9a-f]{64}$' ||
        die "$2 digest is not 64 lowercase hex characters"
}

stamp() {
    _file=$1
    _digest=$2
    [ -f "$_file" ] || die "missing recipe: $_file"
    [ "$(grep -c '^[[:space:]]*sha256sums=' "$_file")" -eq 1 ] ||
        die "$_file must contain exactly one sha256sums= assignment"
    grep -Fxq "sha256sums=('SKIP')" "$_file" ||
        die "$_file has no sha256sums=('SKIP') placeholder"
    _tmp=$(mktemp "${TMPDIR:-/tmp}/v2ray-rs-stamp.XXXXXX") ||
        die "failed to create a temporary file"
    cleanup_files="$cleanup_files $_tmp"
    sed "s/^sha256sums=('SKIP')\$/sha256sums=('$_digest')/" "$_file" >"$_tmp" ||
        die "failed to rewrite $_file"
    mv "$_tmp" "$_file" ||
        die "failed to replace $_file"
    [ "$(grep -c "sha256sums=('$_digest')" "$_file")" -eq 1 ] ||
        die "$_file must contain exactly one sha256sums=('$_digest') line"
    if grep -q 'SKIP' "$_file"; then
        die "$_file still contains SKIP"
    fi
}

[ "$#" -eq 1 ] || die "usage: stamp-release-pkgbuilds.sh <assets-dir>"
assets=$1
[ -d "$assets" ] || die "not a directory: $assets"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY must be set}"
: "${GITHUB_REF_NAME:?GITHUB_REF_NAME must be set}"

sidecar="$assets/v2ray-rs-x86_64-linux.tar.gz.sha256"
[ -f "$sidecar" ] || die "missing tarball digest sidecar: $sidecar"
[ -s "$sidecar" ] || die "empty tarball digest sidecar: $sidecar"
read -r bin_digest _ <"$sidecar"
assert_digest "$bin_digest" "$sidecar"

tarball="$assets/v2ray-rs-x86_64-linux.tar.gz"
[ -f "$tarball" ] ||
    die "missing staged tarball: $tarball; refusing to stamp an unverified digest"
tar_digest=$(sha256sum "$tarball" | awk '{print $1}')
assert_digest "$tar_digest" "$tarball"
[ "$tar_digest" = "$bin_digest" ] ||
    die "$(basename "$tarball") does not match $sidecar"

stamp "$assets/PKGBUILD-bin" "$bin_digest"

archive_url="https://github.com/${GITHUB_REPOSITORY}/archive/${GITHUB_REF_NAME}.tar.gz"
archive=$(mktemp "${TMPDIR:-/tmp}/v2ray-rs-source.XXXXXX.tar.gz")
cleanup_files="$cleanup_files $archive"
curl --fail --location --retry 3 --max-time 120 "$archive_url" -o "$archive" ||
    die "failed to fetch $archive_url"
src_digest=$(sha256sum -b "$archive" | awk '{print $1}')
assert_digest "$src_digest" "$archive_url"
stamp "$assets/PKGBUILD" "$src_digest"
