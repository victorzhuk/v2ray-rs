#!/usr/bin/env bash
# Runs the shared Arch install scriptlet against real filesystem metadata in a
# disposable user namespace: ownership, modes, and file capabilities are set
# and read back by the real tools.
set -uo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/v2ray-rs-hook-ns.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT INT TERM

root="$work/root"
mkdir -p "$root/etc" "$root/case/usr/bin"
printf 'v2ray-rs:x:0:\n' >"$root/etc/group"
printf 'root:x:0:0:root:/root:/bin/sh\nv2ray-rs-bypass:x:0:0::/:/usr/bin/nologin\n' >"$root/etc/passwd"
sed "s|/usr/bin/v2ray-rs-|$root/case/usr/bin/v2ray-rs-|g" \
    "$repo/pkg/archlinux/v2ray-rs.install" >"$root/case/hook.sh"

if ! unshare -rm true 2>/dev/null; then
    printf 'skip: user namespaces are unavailable\n'
    exit 0
fi
# shellcheck disable=SC2016  # $1 belongs to the inner shell
if ! unshare -rm bash -c 'mkdir -p "$1" && mount -t tmpfs tmpfs "$1"' _ "$work/probe" 2>/dev/null; then
    printf 'skip: namespaced tmpfs mounts are unavailable\n'
    exit 0
fi

note() { printf 'namespace: %s\n' "$*"; }
failures=0

note "install"
# shellcheck disable=SC2016  # the script body belongs to the inner shell
if unshare -rm bash -c '
set -euo pipefail
root=$1
mount --bind "$root/etc/group" /etc/group
mount --bind "$root/etc/passwd" /etc/passwd
mount -t tmpfs tmpfs "$root/case/usr/bin"
netctl="$root/case/usr/bin/v2ray-rs-netctl"
run="$root/case/usr/bin/v2ray-rs-run"
printf "#!/bin/sh\nexit 0\n" >"$netctl"
printf "#!/bin/sh\nexit 0\n" >"$run"
chmod 0755 "$netctl" "$run"
. "$root/case/hook.sh"
post_install
[ "$(stat -c %U:%G "$netctl")" = root:v2ray-rs ]
[ "$(stat -c %a "$netctl")" = 750 ]
[ "$(stat -c %U:%G "$run")" = root:v2ray-rs ]
[ "$(stat -c %a "$run")" = 4750 ]
getcap "$netctl" | grep -q cap_net_admin
pre_remove
if getcap "$netctl" | grep -q cap_net_admin; then
    echo "capability survived removal" >&2
    exit 1
fi
post_install
[ "$(stat -c %a "$netctl")" = 750 ]
[ "$(stat -c %a "$run")" = 4750 ]
getcap "$netctl" | grep -q cap_net_admin
' _ "$root"; then
    note "real metadata and reinstall checks passed"
else
    printf 'FAIL: namespaced hook run failed\n' >&2
    failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
    exit 1
fi
