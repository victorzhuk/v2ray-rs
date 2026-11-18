#!/bin/sh
# Fails unless the workspace version and both Arch PKGBUILD pkgver values agree.
#
# Usage: check-pkgbuild-version.sh [--print-version] [repo-root]
#
# --print-version prints the [workspace.package] version and exits 0.
set -eu

print_version=0
if [ "${1:-}" = "--print-version" ]; then
    print_version=1
    shift
fi

repo=${1:-$(cd "$(dirname "$0")/.." && pwd)}

version=$(awk '
    /^[ \t]*\[.*\][ \t]*$/ {
        header = $0
        gsub(/[ \t]/, "", header)
        in_ws = (header == "[workspace.package]")
        next
    }
    in_ws && /^[ \t]*version[ \t]*=[ \t]*"/ {
        sub(/^[ \t]*version[ \t]*=[ \t]*"/, "")
        sub(/".*/, "")
        print
        exit
    }
' "$repo/Cargo.toml")
if [ -z "$version" ]; then
    printf 'check-pkgbuild-version: cannot read [workspace.package].version from %s\n' \
        "$repo/Cargo.toml" >&2
    exit 1
fi

if [ "$print_version" -eq 1 ]; then
    printf '%s\n' "$version"
    exit 0
fi

status=0
for pkgbuild in "$repo/pkg/archlinux/PKGBUILD" "$repo/pkg/archlinux/bin/PKGBUILD"; do
    pkgver=$(sed -n 's/^pkgver=\(.*\)$/\1/p' "$pkgbuild" | head -1 | tr -d '\r')
    case $pkgver in
        '"'*'"')
            pkgver=${pkgver#\"}
            pkgver=${pkgver%\"}
            ;;
        "'"*"'")
            pkgver=${pkgver#\'}
            pkgver=${pkgver%\'}
            ;;
    esac
    if [ "$pkgver" != "$version" ]; then
        printf 'check-pkgbuild-version: %s declares pkgver=%s, expected %s\n' \
            "$pkgbuild" "${pkgver:-<missing>}" "$version" >&2
        status=1
    fi
done
exit $status
