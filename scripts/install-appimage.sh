#!/bin/sh
# shellcheck shell=dash
# shellcheck disable=SC2039  # local is non-POSIX but universally available
#
# v2ray-rs AppImage installer.
#
#   curl -fsSL https://github.com/victorzhuk/v2ray-rs/releases/latest/download/install-appimage.sh | sh
#
# Installs the AppImage under a user-local prefix. The AppImage bundles GTK and
# its translations, so share/locale stays empty on purpose.
#
# Environment:
#   V2RAY_RS_INSTALL_DIR    install prefix (default: $HOME/.local)
#   V2RAY_RS_VERSION        pin a release, e.g. v0.20.2 (default: latest)
#   V2RAY_RS_NO_MODIFY_PATH set to 1 to leave shell rc files alone
#   V2RAY_RS_DOWNLOAD_URL   override the release asset base URL

set -u

# Some ksh builds have no `local`; mksh already aliases it.
has_local() { local _x=0; }
has_local 2>/dev/null || alias local=typeset

APP_NAME="v2ray-rs"
REPO="victorzhuk/v2ray-rs"
ARTIFACT="v2ray-rs-x86_64.AppImage"
INSTALL_URL="https://github.com/victorzhuk/v2ray-rs/releases/latest/download/install-appimage.sh"
DESKTOP_FILE="com.github.v2ray-rs.desktop"

# gen-installer-appimage.sh rewrites these two assignments at release time.
# Everything downstream tests them by *shape*, not against the placeholder
# text: see scripts/install.sh for the reasoning.
APP_VERSION="@VERSION@"
ARTIFACT_SHA256="@SHA256@"

PREFIX="${V2RAY_RS_INSTALL_DIR:-$HOME/.local}"
NO_MODIFY_PATH="${V2RAY_RS_NO_MODIFY_PATH:-0}"
PRINT_QUIET=0
PRINT_VERBOSE=0
DO_UNINSTALL=0

ICON_FILES="icons/hicolor/scalable/apps/com.github.v2ray-rs.svg
icons/hicolor/symbolic/apps/com.github.v2ray-rs-symbolic.svg"

usage() {
    cat <<EOF
$APP_NAME AppImage installer

Downloads the current $APP_NAME AppImage release and installs it under a
prefix, defaulting to \$HOME/.local. Installs the AppImage itself as
bin/v2ray-rs.AppImage, a desktop entry with icons, and env scripts.

The AppImage bundles GTK and its own gettext catalogs, so \$PREFIX/share/locale
stays empty on purpose. The AppImage is installed as a plain file that keeps
its .AppImage suffix: the runtime detects itself through /proc/self/exe, so a
renamed copy breaks it.

USAGE:
    install-appimage.sh [OPTIONS]

OPTIONS:
    -v, --verbose           verbose output
    -q, --quiet             suppress progress output
        --version TAG       install a pinned release, e.g. v0.20.2
        --no-modify-path    do not touch shell rc files
        --uninstall         remove a previous install from the prefix
    -h, --help              print this help

ENVIRONMENT:
    V2RAY_RS_INSTALL_DIR    install prefix (default: \$HOME/.local)
    V2RAY_RS_VERSION        pin a release, e.g. v0.20.2
    V2RAY_RS_NO_MODIFY_PATH set to 1 to leave shell rc files alone
    V2RAY_RS_DOWNLOAD_URL   override the release asset base URL
EOF
}

say() { [ "$PRINT_QUIET" = 1 ] || echo "$1"; }
say_verbose() { [ "$PRINT_VERBOSE" = 1 ] && echo "$1"; return 0; }

warn() {
    [ "$PRINT_QUIET" = 1 ] && return 0
    local _y _r
    _y=$(tput setaf 3 2>/dev/null || echo '')
    _r=$(tput sgr0 2>/dev/null || echo '')
    echo "${_y}warning${_r}: $1" >&2
}

# A warning that --quiet must not be able to hide. Reserved for the cases where
# the user ends up with an unverified download.
insist() {
    local _y _reset
    _y=$(tput setaf 3 2>/dev/null || echo '')
    _reset=$(tput sgr0 2>/dev/null || echo '')
    echo "${_y}warning${_reset}: $1" >&2
}

err() {
    local _r _reset
    _r=$(tput setaf 1 2>/dev/null || echo '')
    _reset=$(tput sgr0 2>/dev/null || echo '')
    echo "${_r}error${_reset}: $1" >&2
    exit 1
}

check_cmd() { command -v "$1" >/dev/null 2>&1; }
need_cmd() { check_cmd "$1" || err "need '$1' (command not found)"; }
ensure() { "$@" || err "command failed: $*"; }

main() {
    parse_args "$@"

    if [ "$DO_UNINSTALL" = 1 ]; then
        uninstall
        return 0
    fi

    need_cmd uname
    need_cmd mktemp
    need_cmd install
    need_cmd rm
    need_cmd sed
    downloader --check

    check_arch
    check_fuse

    local _dir _file _url
    # `ensure` cannot abort from inside $( ): err's exit only kills the subshell.
    _dir=$(mktemp -d) || err "could not create a temporary directory"
    trap 'rm -rf "${_dir:-}"' EXIT INT TERM
    _file="$_dir/$ARTIFACT"
    _url="$(artifact_url)"

    say "downloading $APP_NAME from $_url"
    downloader "$_url" "$_file" || err "download failed: $_url"
    verify_checksum "$_file" "$_url"
    # --appimage-extract executes the image; downloaders do not preserve +x.
    ensure chmod 755 "$_file"

    install_appimage "$_file"
    install_desktop_files "$_file"
    install_license
    rm -rf "$_dir"

    self_check

    refresh_desktop_caches
    [ "$NO_MODIFY_PATH" = 1 ] || setup_path
    print_tun_instructions
}

parse_args() {
    while [ $# -gt 0 ]; do
        case "$1" in
            -v | --verbose) PRINT_VERBOSE=1 ;;
            -q | --quiet) PRINT_QUIET=1 ;;
            --no-modify-path) NO_MODIFY_PATH=1 ;;
            --uninstall) DO_UNINSTALL=1 ;;
            --version)
                [ $# -ge 2 ] || err "--version requires an argument"
                V2RAY_RS_VERSION=${2#v}
                shift
                ;;
            -h | --help) usage; exit 0 ;;
            *) err "unknown option: $1 (see --help)" ;;
        esac
        shift
    done
}

check_arch() {
    local _os _cpu
    _os=$(uname -s)
    _cpu=$(uname -m)
    [ "$_os" = Linux ] || err "$APP_NAME is Linux-only (detected $_os)"
    case "$_cpu" in
        x86_64|amd64) ;;
        *) err "no prebuilt AppImage for $_cpu; build from source: https://github.com/$REPO#from-source" ;;
    esac
}

# The AppImage runtime mounts the squashfs through FUSE. Without it the app
# still runs under APPIMAGE_EXTRACT_AND_RUN=1, so a missing FUSE is only a
# warning, never a failure. GTK is bundled; nothing else is checked.
check_fuse() {
    [ -e /dev/fuse ] && return 0
    if check_cmd ldconfig && ldconfig -p 2>/dev/null | grep -q 'libfuse'; then
        return 0
    fi
    warn "no FUSE support detected (/dev/fuse absent, libfuse not found).
    The AppImage will fall back to APPIMAGE_EXTRACT_AND_RUN=1, which extracts
    itself to a temporary directory on every start and is slower."
}

# A substituted version starts with a digit; the placeholder does not.
is_pinned_version() {
    case "$APP_VERSION" in
        [0-9]*) return 0 ;;
        *) return 1 ;;
    esac
}

# A substituted digest is exactly 64 lowercase hex characters.
is_pinned_sha256() {
    case "$ARTIFACT_SHA256" in
        "" | *[!0-9a-f]*) return 1 ;;
    esac
    [ "${#ARTIFACT_SHA256}" -eq 64 ]
}

artifact_url() {
    if [ -n "${V2RAY_RS_DOWNLOAD_URL:-}" ]; then
        echo "$V2RAY_RS_DOWNLOAD_URL"
    elif [ -n "${V2RAY_RS_VERSION:-}" ]; then
        echo "https://github.com/$REPO/releases/download/v$V2RAY_RS_VERSION/$ARTIFACT"
    elif is_pinned_version; then
        echo "https://github.com/$REPO/releases/download/v$APP_VERSION/$ARTIFACT"
    else
        echo "https://github.com/$REPO/releases/latest/download/$ARTIFACT"
    fi
}

# The baked digest describes exactly one artifact, so it only applies when
# nothing has redirected the download somewhere else.
baked_digest_applies() {
    is_pinned_sha256 || return 1
    [ -z "${V2RAY_RS_DOWNLOAD_URL:-}" ] || return 1
    [ -z "${V2RAY_RS_VERSION:-}" ] || [ "${V2RAY_RS_VERSION:-}" = "$APP_VERSION" ]
}

# Prefers the digest baked into this script, since that one was fixed when the
# script was published. Falls back to the `.sha256` published beside the
# AppImage, which only proves the download was not corrupted in transit.
resolve_expected_sha256() {
    local _url="$1" _side="$2.sha256" _want

    if baked_digest_applies; then
        echo "$ARTIFACT_SHA256"
        return 0
    fi

    if downloader "$_url.sha256" "$_side" 2>/dev/null; then
        _want=$(awk 'NR==1 {print $1}' "$_side")
        case "$_want" in
            "" | *[!0-9a-f]*) _want="" ;;
        esac
        [ "${#_want}" -eq 64 ] || _want=""
    fi

    [ -n "${_want:-}" ] || return 1
    echo "$_want"
}

verify_checksum() {
    local _file="$1" _url="$2" _want _got

    if ! check_cmd sha256sum; then
        insist "sha256sum not found; the download was NOT verified"
        return 0
    fi

    _want=$(resolve_expected_sha256 "$_url" "$_file") || {
        insist "no checksum available for this download; it was NOT verified"
        return 0
    }

    _got=$(sha256sum -b "$_file" | awk '{print $1}')
    [ -n "$_got" ] || err "could not hash $_file"
    [ "$_got" = "$_want" ] || err "checksum mismatch
    want: $_want
    got:  $_got"

    if baked_digest_applies; then
        say_verbose "checksum ok (pinned in this installer)"
    else
        say "checksum ok (from $ARTIFACT.sha256; not pinned by this installer)"
    fi
}

install_appimage() {
    local _src="$1"

    say "installing to $PREFIX"
    # Named v2ray-rs.AppImage, never v2ray-rs: the tarball installer owns
    # bin/v2ray-rs, and the AppImage runtime needs the suffix to detect itself.
    ensure install -Dm755 "$_src" "$PREFIX/bin/v2ray-rs.AppImage"
    say "  bin/v2ray-rs.AppImage"
}

# linuxdeploy bakes Exec=v2ray-rs, which only resolves when $PREFIX/bin is on
# $PATH -- it is not, from a desktop launcher. Rewrite exactly the Exec= line
# (and drop TryExec=) and leave every other line untouched. Re-extracted and
# rewritten on every install so a version bump cannot leave a stale Exec.
install_desktop_files() {
    local _appimage="$1" _sq _desktop _png _icon
    _sq="$_appimage.exdir/squashfs-root"
    mkdir -p "$_appimage.exdir"

    # --appimage-extract needs no FUSE and always writes to ./squashfs-root.
    (cd "$_appimage.exdir" && "$_appimage" --appimage-extract 'usr/share/applications/*') \
        || err "could not extract the desktop entry from the AppImage"
    (cd "$_appimage.exdir" && "$_appimage" --appimage-extract 'usr/share/icons/*') \
        || err "could not extract the icons from the AppImage"

    _desktop="$_sq/usr/share/applications/$DESKTOP_FILE"
    [ -f "$_desktop" ] || err "AppImage is missing usr/share/applications/$DESKTOP_FILE"
    ensure sed -e "s|^Exec=.*|Exec=$PREFIX/bin/v2ray-rs.AppImage %U|" \
        -e "/^TryExec=/d" "$_desktop" >"$_dir/$DESKTOP_FILE"
    ensure install -Dm644 "$_dir/$DESKTOP_FILE" "$PREFIX/share/applications/$DESKTOP_FILE"
    say "  share/applications/$DESKTOP_FILE (Exec rewritten)"

    _png="$_sq/usr/share/icons/hicolor/256x256/apps/com.github.v2ray-rs.png"
    if [ -f "$_png" ]; then
        ensure install -Dm644 "$_png" "$PREFIX/share/icons/hicolor/256x256/apps/com.github.v2ray-rs.png"
    else
        # The one the desktop entry's Icon= resolves to.
        warn "AppImage has no 256x256 PNG icon; the desktop entry may show a generic icon"
    fi

    for _icon in $ICON_FILES; do
        if [ -f "$_sq/usr/share/$_icon" ]; then
            ensure install -Dm644 "$_sq/usr/share/$_icon" "$PREFIX/share/$_icon"
        else
            say_verbose "  optional icon absent in this AppImage: share/$_icon"
        fi
    done
}

# The AppImage carries no top-level LICENSE usable without FUSE, so the
# repository copy next to this script is used when it is reachable. A piped
# install has no repository around it and simply skips.
install_license() {
    local _src
    _src=$(dirname "$0")/../LICENSE
    [ -f "$_src" ] || _src=$(dirname "$0")/LICENSE
    if [ -f "$_src" ]; then
        ensure install -Dm644 "$_src" "$PREFIX/share/licenses/$APP_NAME/LICENSE"
    else
        say_verbose "repository LICENSE not found; skipping share/licenses/$APP_NAME"
    fi
}

# Runs the installed AppImage once: a broken download or an unusable runtime
# fails here instead of on first launch.
self_check() {
    local _bin="$PREFIX/bin/v2ray-rs.AppImage"
    if "$_bin" --help >/dev/null 2>&1; then
        say "self check ok ($_bin --help)"
        return 0
    fi
    if APPIMAGE_EXTRACT_AND_RUN=1 "$_bin" --help >/dev/null 2>&1; then
        say "self check ok under APPIMAGE_EXTRACT_AND_RUN=1 (FUSE unavailable)"
        return 0
    fi
    err "self check failed: $_bin --help did not succeed"
}

uninstall() {
    local _icon

    say "removing $APP_NAME AppImage install from $PREFIX"
    rm -f "$PREFIX/bin/v2ray-rs.AppImage"
    rm -f "$PREFIX/share/applications/$DESKTOP_FILE"
    rm -f "$PREFIX/share/icons/hicolor/256x256/apps/com.github.v2ray-rs.png"
    for _icon in $ICON_FILES; do
        rm -f "$PREFIX/share/$_icon"
    done
    rm -rf "$PREFIX/share/licenses/$APP_NAME"
    rm -f "$PREFIX/env" "$PREFIX/env.fish"
    rm -f "$HOME/.config/fish/conf.d/$APP_NAME.fish"

    # Refresh first so the caches drop our entry, then discard them only where
    # the prefix holds nothing else -- another application may share it.
    refresh_desktop_caches
    prune_cache "$PREFIX/share/applications" mimeinfo.cache
    prune_cache "$PREFIX/share/icons/hicolor" icon-theme.cache
    say "done. Shell rc files were not modified; drop the '$PREFIX/env' line if you added one."
    say "A helper relocated by Grant TUN privileges is deliberately left alone:"
    say "    sudo rm /usr/local/lib/v2ray-rs/v2ray-rs-netctl   # if you granted it"
}

# Removes a generated cache when it is the only thing left under dir, then the
# now-empty directory tree. Leaves a shared prefix untouched.
prune_cache() {
    local _dir="$1" _cache="$2"

    [ -d "$_dir" ] || return 0
    if [ -z "$(find "$_dir" -type f ! -name "$_cache" -print -quit 2>/dev/null)" ]; then
        rm -f "$_dir/$_cache"
        find "$_dir" -type d -empty -delete 2>/dev/null
    fi
    return 0
}

refresh_desktop_caches() {
    check_cmd update-desktop-database && update-desktop-database "$PREFIX/share/applications" >/dev/null 2>&1
    check_cmd gtk-update-icon-cache && gtk-update-icon-cache -qtf "$PREFIX/share/icons/hicolor" >/dev/null 2>&1
    return 0
}

setup_path() {
    case ":$PATH:" in
        *":$PREFIX/bin:"*) say_verbose "$PREFIX/bin already on PATH"; return 0 ;;
    esac

    local _expr _env _env_expr _fish_env _fish_env_expr
    _expr=$(replace_home "$PREFIX/bin")
    _env="$PREFIX/env"
    _env_expr=$(replace_home "$_env")
    _fish_env="$_env.fish"
    _fish_env_expr="$_env_expr.fish"

    write_env_sh "$_expr" "$_env"
    write_env_fish "$_expr" "$_fish_env"

    local _touched=0 _zdot
    _zdot="${ZDOTDIR:-$HOME}"
    for _rc in "$HOME/.profile" "$HOME/.bashrc" "$_zdot/.zshrc"; do
        add_source_line "$_rc" "[ -f \"$_env_expr\" ] && . \"$_env_expr\"" "$_rc" && _touched=1
    done
    # Only ever appended to, never created: bash reads ~/.bash_profile *instead
    # of* ~/.profile, so creating one would silently orphan the user's existing
    # login configuration.
    if [ -f "$HOME/.bash_profile" ]; then
        add_source_line "$HOME/.bash_profile" "[ -f \"$_env_expr\" ] && . \"$_env_expr\"" ".bash_profile" && _touched=1
    fi
    if [ -d "$HOME/.config/fish" ]; then
        mkdir -p "$HOME/.config/fish/conf.d"
        add_source_line "$HOME/.config/fish/conf.d/$APP_NAME.fish" \
            "test -f \"$_fish_env_expr\" && source \"$_fish_env_expr\"" "fish" && _touched=1
    fi

    if [ "$_touched" = 1 ]; then
        say ""
        say "To use $APP_NAME now, restart your shell or run:"
        say "    source $_env_expr        (sh, bash, zsh)"
        say "    source $_fish_env_expr   (fish)"
    fi
}

# Writes the source line unless the file already has it. Returns 0 when the file
# was modified so the caller knows whether to tell the user to reload.
add_source_line() {
    local _target="$1" _line="$2" _label="$3"

    if [ -f "$_target" ] && grep -Fq "$_line" "$_target"; then
        say_verbose "$_label already sources the env script"
        return 1
    fi
    say_verbose "adding env script to $_target"
    printf '\n%s\n' "$_line" >>"$_target" || return 1
    return 0
}

write_env_sh() {
    local _dir_expr="$1" _path="$2"
    cat >"$_path" <<EOF || err "cannot write $_path"
#!/bin/sh
case ":\${PATH}:" in
    *:"$_dir_expr":*) ;;
    *) export PATH="$_dir_expr:\$PATH" ;;
esac
EOF
}

write_env_fish() {
    local _dir_expr="$1" _path="$2"
    cat >"$_path" <<EOF || err "cannot write $_path"
if not contains "$_dir_expr" \$PATH
    set -x PATH "$_dir_expr" \$PATH
end
EOF
}

# Late-bind $HOME so the rc line keeps working if the home directory moves.
# Done with a prefix strip rather than sed: a comma or a regex metacharacter in
# $HOME would break an s,,, expression, and an empty result would produce an
# empty PATH element, which means the current directory.
replace_home() {
    # shellcheck disable=SC2016  # single quotes are the point: no expansion
    if [ -n "${HOME:-}" ] && [ "${1#"$HOME"/}" != "$1" ]; then
        printf '$HOME/%s\n' "${1#"$HOME"/}"
    else
        printf '%s\n' "$1"
    fi
}

print_tun_instructions() {
    if check_cmd getent && getent group "$APP_NAME" >/dev/null 2>&1; then
        say ""
        say "System group '$APP_NAME' present; TUN mode is reachable through the app."
    else
        cat <<EOF

Installed for your user. Everything except TUN mode works as-is.

TUN mode needs the system group '$APP_NAME', which a user-local install
cannot create. Ask an administrator to run:

    sudo groupadd --system $APP_NAME
    sudo usermod -aG $APP_NAME $USER

then log out and back in for the group to take effect.

The setuid bypass wrapper (v2ray-rs-run) does not exist in the AppImage, so
'Run with bypass' is permanently unavailable from this install. TUN mode works
through the in-app 'Grant TUN privileges' flow, which relocates the bundled
helper to /usr/local/lib/$APP_NAME/v2ray-rs-netctl (the squashfs is mounted
nosuid, so capabilities cannot live inside it). Re-run the grant after every
AppImage update.

For the supported TUN path, use the system-wide tarball install:

    curl -fsSL $INSTALL_URL | sudo env V2RAY_RS_INSTALL_DIR=/usr/local sh

or a distribution package (on Arch: yay -S v2ray-rs-bin).

This script never runs sudo and modifies nothing outside $PREFIX except the
shell rc source lines above. You also need a backend: v2ray, xray, or sing-box.
EOF
    fi
}

downloader() {
    local _dld
    if check_cmd curl; then
        _dld=curl
    elif check_cmd wget; then
        _dld=wget
    else
        _dld='curl or wget'
    fi

    if [ "$1" = --check ]; then
        need_cmd "$_dld"
    elif [ "$_dld" = curl ]; then
        curl -sSfL "$1" -o "$2"
    elif [ "$_dld" = wget ]; then
        wget -q "$1" -O "$2"
    else
        err "no downloader available"
    fi
}

main "$@" || exit 1
