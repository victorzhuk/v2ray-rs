#!/bin/sh
# shellcheck shell=dash
# shellcheck disable=SC2039  # local is non-POSIX but universally available
#
# v2ray-rs installer.
#
#   curl -fsSL https://github.com/victorzhuk/v2ray-rs/releases/latest/download/install.sh | sh
#
# Environment:
#   V2RAY_RS_INSTALL_DIR    install prefix (default: $HOME/.local)
#   V2RAY_RS_VERSION        pin a release, e.g. 0.16.1 (default: latest)
#   V2RAY_RS_NO_MODIFY_PATH set to 1 to leave shell rc files alone
#   V2RAY_RS_DOWNLOAD_URL   override the release asset base URL

set -u

# Some ksh builds have no `local`; mksh already aliases it.
has_local() {
    # shellcheck disable=SC2034
    local _probe
}
has_local 2>/dev/null || alias local=typeset

APP_NAME="v2ray-rs"
REPO="victorzhuk/v2ray-rs"
ARTIFACT="v2ray-rs-x86_64-linux.tar.gz"
INSTALL_URL="https://github.com/victorzhuk/v2ray-rs/releases/latest/download/install.sh"

# gen-installer.sh rewrites these two assignments at release time. Everything
# downstream tests them by *shape* rather than comparing against the placeholder
# text: a blanket substitution would otherwise rewrite the comparisons too and
# turn them into tautologies, silently disabling both pinning and verification.
# Shape checks also reject a substitution that produced something malformed.
APP_VERSION="@VERSION@"
ARTIFACT_SHA256="@SHA256@"

# The GUI links GTK dynamically; these are the versions relm4's gnome_48 feature
# compiles against, and an older host aborts at startup rather than at link time.
GTK_MIN="4.18"
ADW_MIN="1.7"

PREFIX="${V2RAY_RS_INSTALL_DIR:-$HOME/.local}"
NO_MODIFY_PATH="${V2RAY_RS_NO_MODIFY_PATH:-0}"
PRINT_QUIET=0
PRINT_VERBOSE=0
DO_UNINSTALL=0

BINS="v2ray-rs v2ray-rs-netctl v2ray-rs-run"
SHARE_FILES="applications/com.github.v2ray-rs.desktop
icons/hicolor/scalable/apps/com.github.v2ray-rs.svg
icons/hicolor/symbolic/apps/com.github.v2ray-rs-symbolic.svg
icons/hicolor/256x256/apps/com.github.v2ray-rs.png
locale/en_US/LC_MESSAGES/v2ray-rs.mo
locale/ru_RU/LC_MESSAGES/v2ray-rs.mo"

usage() {
    cat <<EOF
$APP_NAME installer

Downloads the current $APP_NAME release and installs it under a prefix,
defaulting to \$HOME/.local. Installs three binaries, a desktop entry, icons,
and translations.

USAGE:
    install.sh [OPTIONS]

OPTIONS:
    -v, --verbose           verbose output
    -q, --quiet             suppress progress output
        --no-modify-path    do not touch shell rc files
        --uninstall         remove a previous install from the prefix
    -h, --help              print this help

ENVIRONMENT:
    V2RAY_RS_INSTALL_DIR    install prefix (default: \$HOME/.local)
    V2RAY_RS_VERSION        pin a release, e.g. 0.16.1
    V2RAY_RS_NO_MODIFY_PATH set to 1 to leave shell rc files alone
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
    need_cmd tar
    need_cmd install
    need_cmd rm
    downloader --check

    check_arch
    check_runtime_deps

    local _dir _file _url
    # `ensure` cannot abort from inside $( ): err's exit only kills the subshell.
    _dir=$(mktemp -d) || err "could not create a temporary directory"
    trap 'rm -rf "${_dir:-}"' EXIT INT TERM
    _file="$_dir/$ARTIFACT"
    _url="$(artifact_url)"

    say "downloading $APP_NAME from $_url"
    downloader "$_url" "$_file" || err "download failed: $_url"
    verify_checksum "$_file" "$_url"

    ensure tar xf "$_file" --no-same-owner --strip-components 1 -C "$_dir"
    install_tree "$_dir"
    rm -rf "$_dir"

    refresh_desktop_caches
    [ "$NO_MODIFY_PATH" = 1 ] || setup_path
    print_tun_instructions
}

parse_args() {
    for arg in "$@"; do
        case "$arg" in
            -h|--help) usage; exit 0 ;;
            -v|--verbose) PRINT_VERBOSE=1 ;;
            -q|--quiet) PRINT_QUIET=1 ;;
            --no-modify-path) NO_MODIFY_PATH=1 ;;
            --uninstall) DO_UNINSTALL=1 ;;
            *) err "unknown option $arg (try --help)" ;;
        esac
    done
}

check_arch() {
    local _os _cpu
    _os=$(uname -s)
    _cpu=$(uname -m)
    [ "$_os" = Linux ] || err "$APP_NAME is Linux-only (detected $_os)"
    case "$_cpu" in
        x86_64|amd64) ;;
        *) err "no prebuilt binaries for $_cpu; build from source: https://github.com/$REPO#from-source" ;;
    esac
}

# GTK and libadwaita stay dynamically linked, so a host below the floor cannot
# run the tarball at all. Fail here with an actionable message instead of
# leaving the user with a loader error on first launch.
check_runtime_deps() {
    local _missing=""

    if check_cmd pkg-config; then
        pkg-config --atleast-version="$GTK_MIN" gtk4 2>/dev/null || _missing="$_missing gtk4>=$GTK_MIN"
        pkg-config --atleast-version="$ADW_MIN" libadwaita-1 2>/dev/null || _missing="$_missing libadwaita>=$ADW_MIN"
    elif check_cmd ldconfig; then
        # Without pkg-config only presence is checkable, not the version.
        ldconfig -p 2>/dev/null | grep -q 'libgtk-4\.so' || _missing="$_missing gtk4"
        ldconfig -p 2>/dev/null | grep -q 'libadwaita-1\.so' || _missing="$_missing libadwaita"
    else
        warn "cannot verify GTK $GTK_MIN / libadwaita $ADW_MIN are installed (no pkg-config or ldconfig)"
        return 0
    fi

    [ -z "$_missing" ] && return 0

    cat >&2 <<EOF
error: missing runtime dependencies:$_missing

  Debian/Ubuntu   sudo apt install libgtk-4-1 libadwaita-1-0
  Fedora          sudo dnf install gtk4 libadwaita
  Arch            sudo pacman -S gtk4 libadwaita

  GTK $GTK_MIN and libadwaita $ADW_MIN are the minimum. On an older distribution
  use the AppImage instead, which bundles them:

    https://github.com/$REPO/releases/latest
EOF
    exit 1
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
# tarball, which only proves the download was not corrupted in transit.
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

install_tree() {
    local _src="$1" _bin _rel

    say "installing to $PREFIX"
    for _bin in $BINS; do
        [ -f "$_src/bin/$_bin" ] || err "archive is missing bin/$_bin"
        ensure install -Dm755 "$_src/bin/$_bin" "$PREFIX/bin/$_bin"
        say "  bin/$_bin"
    done

    for _rel in $SHARE_FILES; do
        [ -f "$_src/share/$_rel" ] || err "archive is missing share/$_rel"
        ensure install -Dm644 "$_src/share/$_rel" "$PREFIX/share/$_rel"
    done

    ensure install -Dm644 "$_src/LICENSE" "$PREFIX/share/licenses/$APP_NAME/LICENSE"
    say "everything's installed"
}

uninstall() {
    local _bin _rel

    say "removing $APP_NAME from $PREFIX"
    for _bin in $BINS; do
        rm -f "$PREFIX/bin/$_bin"
    done
    for _rel in $SHARE_FILES; do
        rm -f "$PREFIX/share/$_rel"
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
    say "Privileged helpers, if you granted them, are unaffected:"
    say "    sudo setcap -r $PREFIX/bin/v2ray-rs-netctl   # before this uninstall"
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
    if [ -n "${HOME:-}" ] && [ "${1#"$HOME"/}" != "$1" ]; then
        printf '$HOME/%s\n' "${1#"$HOME"/}"
    else
        printf '%s\n' "$1"
    fi
}

print_tun_instructions() {
    local _bindir="$PREFIX/bin"

    if is_nosuid "$_bindir"; then
        insist "$_bindir is on a filesystem mounted nosuid. File capabilities and
         the setuid bit are ignored there, so TUN mode cannot work from this
         prefix. Use a prefix on a normal mount, or a distribution package."
        return 0
    fi

    # v2ray-rs-run is setuid-*root*. Chmod 4750 on a file the invoking user owns
    # makes it setuid to that same user, which is a no-op -- so the recipe below
    # is only meaningful when the install itself ran as root.
    if [ "$(id -u)" -ne 0 ]; then
        cat <<EOF

Installed for your user. Everything except TUN mode works as-is.

TUN mode needs helpers owned by root, which a home-directory install cannot
provide. For it, either install system-wide:

    curl -fsSL $INSTALL_URL | sudo env V2RAY_RS_INSTALL_DIR=/usr/local sh

or use a distribution package (on Arch: yay -S v2ray-rs-bin), which sets the
helper privileges up for you.

You also need a backend: v2ray, xray, or sing-box.
EOF
        return 0
    fi

    cat <<EOF

Installed system-wide. To finish enabling TUN mode, run:

    groupadd --system v2ray-rs
    useradd --system --no-create-home --shell /usr/sbin/nologin v2ray-rs-bypass
    chgrp v2ray-rs $_bindir/v2ray-rs-netctl $_bindir/v2ray-rs-run
    chmod 0750 $_bindir/v2ray-rs-netctl
    chmod 4750 $_bindir/v2ray-rs-run
    setcap 'cap_net_admin+ep' $_bindir/v2ray-rs-netctl
    usermod -aG v2ray-rs <your-user>

Log out and back in for the group to take effect.

You also need a backend: v2ray, xray, or sing-box.
EOF
}

# Longest matching mount point in /proc/self/mounts wins, mirroring how the app
# itself decides whether a prefix can hold file capabilities.
is_nosuid() {
    local _path="$1" _best="" _best_opts="" _mp _opts

    [ -r /proc/self/mounts ] || return 1
    # Mount points are escaped in /proc/self/mounts; a space is \040. Without
    # un-escaping, a path under such a mount silently falls back to / and the
    # nosuid warning is never printed.
    while read -r _ _mp _ _opts _; do
        case "$_mp" in
            *'\'*) _mp=$(printf '%b' "$(printf '%s' "$_mp" | sed 's/\\\([0-7][0-7][0-7]\)/\\0\1/g')") ;;
        esac
        if [ "$_mp" = / ] || [ "$_path" = "$_mp" ] \
            || [ "${_path#"$_mp"/}" != "$_path" ]; then
            if [ "${#_mp}" -ge "${#_best}" ]; then
                _best="$_mp"
                _best_opts="$_opts"
            fi
        fi
    done </proc/self/mounts

    [ -n "$_best" ] || return 1
    case ",$_best_opts," in
        *,nosuid,*) return 0 ;;
    esac
    return 1
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
