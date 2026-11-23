#!/usr/bin/env bash
# Stages the release tree consumed by install.sh, the AppImage build, and the
# v2ray-rs-bin package. The layout mirrors what the app resolves at runtime:
# helpers are siblings of the GUI binary, locale sits at ../share/locale.
set -euo pipefail

usage() {
    cat <<'EOF'
usage: stage-dist.sh --version VER --ui PATH --netctl PATH --run PATH
                     --antidpi PATH --out DIR [--name NAME]

Populates OUT/NAME with the release tree. NAME defaults to
v2ray-rs-VER-x86_64-linux.
EOF
}

version=
ui=
netctl=
run=
antidpi=
out=
name=

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version) version=$2; shift 2 ;;
        --ui)      ui=$2;      shift 2 ;;
        --netctl)  netctl=$2;  shift 2 ;;
        --run)     run=$2;     shift 2 ;;
        --antidpi) antidpi=$2; shift 2 ;;
        --out)     out=$2;     shift 2 ;;
        --name)    name=$2;    shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

for var in version ui netctl out; do
    if [[ -z ${!var} ]]; then
        echo "missing --${var}" >&2
        exit 2
    fi
done
if [[ -z $run ]]; then
    echo "missing --run" >&2
    exit 2
fi
if [[ -z $antidpi ]]; then
    echo "missing --antidpi" >&2
    exit 2
fi

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
name=${name:-v2ray-rs-${version}-x86_64-linux}
root=${out}/${name}

rm -rf "$root"
install -Dm755 "$ui" "$root/bin/v2ray-rs"
install -Dm755 "$netctl" "$root/bin/v2ray-rs-netctl"
# Shipped 0755; the setuid bit is applied at install time by the package hook,
# never baked into a tarball.
install -Dm755 "$run" "$root/bin/v2ray-rs-run"
# Anti-DPI engine; shipped 0755, the capability grant happens at install time.
install -Dm755 "$antidpi" "$root/bin/v2ray-rs-antidpi"

install -Dm644 "$repo/assets/com.github.v2ray-rs.desktop" \
    "$root/share/applications/com.github.v2ray-rs.desktop"
install -Dm644 "$repo/crates/ui/icons/hicolor/scalable/apps/com.github.v2ray-rs.svg" \
    "$root/share/icons/hicolor/scalable/apps/com.github.v2ray-rs.svg"
install -Dm644 "$repo/crates/ui/icons/hicolor/symbolic/apps/com.github.v2ray-rs-symbolic.svg" \
    "$root/share/icons/hicolor/symbolic/apps/com.github.v2ray-rs-symbolic.svg"
install -Dm644 "$repo/assets/v2ray-rs.png" \
    "$root/share/icons/hicolor/256x256/apps/com.github.v2ray-rs.png"

# Compile the catalogs from the tracked .po files: a stale committed .mo must
# never ship. msgunfmt verifies a new key survived compilation.
for tool in msgfmt msgunfmt; do
    command -v "$tool" >/dev/null 2>&1 ||
        { echo "missing $tool: gettext is required to stage the translation catalogs" >&2; exit 2; }
done
for lang in en_US ru_RU; do
    po="$repo/locale/$lang/LC_MESSAGES/v2ray-rs.po"
    mo="$root/share/locale/$lang/LC_MESSAGES/v2ray-rs.mo"
    tmp_mo="$root/.v2ray-rs-$lang.mo"
    txt="$root/.v2ray-rs-$lang.txt"
    msgfmt -o "$tmp_mo" "$po"
    install -Dm644 "$tmp_mo" "$mo"
    rm -f "$tmp_mo"
    msgunfmt "$mo" > "$txt" ||
        { echo "staged $lang catalog lost the 'Anti-DPI engine' key" >&2; exit 2; }
    grep -q 'msgid "Anti-DPI engine"' "$txt" ||
        { echo "staged $lang catalog lost the 'Anti-DPI engine' key" >&2; exit 2; }
    rm -f "$txt"
done

install -Dm644 "$repo/LICENSE" "$root/LICENSE"
install -Dm644 "$repo/README.md" "$root/README.md"

echo "$root"
