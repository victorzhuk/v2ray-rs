#!/usr/bin/env bash
# Exercises the shared Arch install scriptlet against a temporary helper tree.
# System account tools and privileged metadata operations are stubbed so the
# scriptlet's control flow and cleanup can be checked without touching the host.
set -uo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/v2ray-rs-hook.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT INT TERM

failures=0
pass() { printf 'ok: %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; failures=$((failures + 1)); }
note() { printf '\n== %s\n' "$*"; }

real_stat=$(command -v stat)
stub="$work/stub"
mkdir -p "$stub"

cat >"$stub/getent" <<'EOF'
#!/bin/sh
[ "$1" = group ] || exit 2
grep -qx "group:$2" "$IDENTITY_LEDGER" || exit 2
printf '%s:x:1000:\n' "$2"
EOF

cat >"$stub/groupadd" <<'EOF'
#!/bin/sh
for name in "$@"; do :; done
printf 'group:%s\n' "$name" >>"$IDENTITY_LEDGER"
printf 'groupadd %s\n' "$name" >>"$CALL_LOG"
EOF

cat >"$stub/useradd" <<'EOF'
#!/bin/sh
for name in "$@"; do :; done
printf 'user:%s\n' "$name" >>"$IDENTITY_LEDGER"
printf 'useradd %s\n' "$name" >>"$CALL_LOG"
EOF

cat >"$stub/userdel" <<'EOF'
#!/bin/sh
for name in "$@"; do :; done
printf 'userdel %s\n' "$name" >>"$CALL_LOG"
printf 'userdel %s\n' "$name" >>"$UNEXPECTED_LOG"
exit 0
EOF

cat >"$stub/groupdel" <<'EOF'
#!/bin/sh
for name in "$@"; do :; done
printf 'groupdel %s\n' "$name" >>"$CALL_LOG"
printf 'groupdel %s\n' "$name" >>"$UNEXPECTED_LOG"
exit 0
EOF

cat >"$stub/id" <<'EOF'
#!/bin/sh
grep -qx "user:$1" "$IDENTITY_LEDGER" || exit 1
printf 'uid=999(%s) gid=999\n' "$1"
EOF

cat >"$stub/chown" <<'EOF'
#!/bin/sh
[ "${CHOWN_FAIL:-0}" = 1 ] && exit 1
[ "${CHOWN_NOOP:-0}" = 1 ] && exit 0
owner=$1
shift
for file in "$@"; do
    printf '%s\n' "$owner" >"$file.owner"
    rm -f "$file.caps"
done
printf 'chown %s %s\n' "$owner" "$*" >>"$CALL_LOG"
EOF

cat >"$stub/stat" <<'EOF'
#!/bin/sh
[ "$1" = -c ] || exit 2
format=$2
file=$3
case "$format" in
    '%U:%G')
        if [ -f "$file.owner" ]; then cat "$file.owner"; else printf 'root:root\n'; fi
        ;;
    '%a')
        @REAL_STAT@ -c %a "$file"
        ;;
    *)
        @REAL_STAT@ -c "$format" "$file"
        ;;
esac
EOF
sed -i "s|@REAL_STAT@|$real_stat|g" "$stub/stat"

cat >"$stub/getcap" <<'EOF'
#!/bin/sh
[ -f "$1.caps" ] && cat "$1.caps"
exit 0
EOF

cat >"$stub/setcap" <<'EOF'
#!/bin/sh
if [ "${SETCAP_FAIL:-0}" = 1 ]; then
    printf 'setcap failed\n' >>"$CALL_LOG"
    exit 1
fi
if [ "$1" = -r ]; then
    shift
    for file in "$@"; do rm -f "$file.caps"; done
    printf 'setcap -r %s\n' "$*" >>"$CALL_LOG"
    exit 0
fi
capabilities=$1
shift
for file in "$@"; do
    printf '%s cap_net_admin=ep\n' "$file" >"$file.caps"
done
printf 'setcap %s %s\n' "$capabilities" "$*" >>"$CALL_LOG"
EOF
chmod 0755 "$stub"/*

setup_case() {
    local name=$1 root="$work/$1" bin

    mkdir -p "$root/usr/bin"
    for bin in v2ray-rs-netctl v2ray-rs-run; do
        printf '#!/bin/sh\nexit 0\n' >"$root/usr/bin/$bin"
        chmod 0755 "$root/usr/bin/$bin"
    done
    sed "s|/usr/bin/v2ray-rs-|$root/usr/bin/v2ray-rs-|g" \
        "$repo/pkg/archlinux/v2ray-rs.install" >"$root/hook.sh"
    : >"$root/identities"
    : >"$root/calls"
    : >"$root/unexpected"
    : >"$root/out"
}

run_hook() {
    local name=$1 function=$2
    shift 2
    # shellcheck disable=SC2016  # $1 and $2 are the inner shell's arguments
    env PATH="$stub:$PATH" \
        IDENTITY_LEDGER="$work/$name/identities" \
        CALL_LOG="$work/$name/calls" \
        UNEXPECTED_LOG="$work/$name/unexpected" \
        CHOWN_FAIL=0 CHOWN_NOOP=0 SETCAP_FAIL=0 "$@" \
        bash -c '. "$1"; "$2"' _ "$work/$name/hook.sh" "$function" \
        >"$work/$name/out" 2>&1
}

mode_of() { "$real_stat" -c %a "$1"; }
owner_of() {
    if [ -f "$1.owner" ]; then cat "$1.owner"; else printf 'root:root\n'; fi
}
has_cap() { grep -q 'cap_net_admin' "$1.caps" 2>/dev/null; }
identity_count() { grep -c "^$1:" "$work/$2/identities" 2>/dev/null || true; }
call_count() { grep -c "^$1" "$work/$2/calls" 2>/dev/null || true; }
output_has() { grep -q "$1" "$work/$2/out"; }

assert_helpers() {
    local name=$1 netctl="$work/$1/usr/bin/v2ray-rs-netctl" run="$work/$1/usr/bin/v2ray-rs-run"

    [ "$(mode_of "$netctl")" = 750 ] || fail "$name: netctl mode is $(mode_of "$netctl"), want 750"
    [ "$(mode_of "$run")" = 4750 ] || fail "$name: wrapper mode is $(mode_of "$run"), want 4750"
    [ "$(owner_of "$netctl")" = root:v2ray-rs ] ||
        fail "$name: netctl owner is $(owner_of "$netctl"), want root:v2ray-rs"
    [ "$(owner_of "$run")" = root:v2ray-rs ] ||
        fail "$name: wrapper owner is $(owner_of "$run"), want root:v2ray-rs"
    has_cap "$netctl" || fail "$name: netctl lost cap_net_admin"
}

assert_unprivileged() {
    local name=$1 netctl="$work/$1/usr/bin/v2ray-rs-netctl" run="$work/$1/usr/bin/v2ray-rs-run"

    [ "$(mode_of "$netctl")" = 750 ] || fail "$name: netctl mode is $(mode_of "$netctl"), want 750"
    [ "$(mode_of "$run")" = 750 ] || fail "$name: wrapper mode is $(mode_of "$run"), want 750"
    has_cap "$netctl" && fail "$name: netctl kept cap_net_admin"
    return 0
}

note "install"
setup_case install
if run_hook install post_install; then
    assert_helpers install
    [ "$(identity_count group install)" = 1 ] || fail "install: group was not created once"
    [ "$(identity_count user install)" = 1 ] || fail "install: user was not created once"
    [ "$(call_count groupadd install)" = 1 ] || fail "install: unexpected groupadd count"
    [ "$(call_count useradd install)" = 1 ] || fail "install: unexpected useradd count"
    output_has "usermod -aG v2ray-rs" install || fail "install: group instructions missing"
    pass "install provisions restricted helpers"
else
    fail "install: post_install failed: $(tail -n 4 "$work/install/out" | tr '\n' ' ')"
fi

note "upgrade"
if run_hook install post_upgrade; then
    assert_helpers install
    [ "$(call_count groupadd install)" = 1 ] || fail "upgrade: groupadd ran again"
    [ "$(call_count useradd install)" = 1 ] || fail "upgrade: useradd ran again"
    pass "upgrade reapplies helper privileges without new identities"
else
    fail "upgrade: post_upgrade failed: $(tail -n 4 "$work/install/out" | tr '\n' ' ')"
fi

note "setcap failure"
if run_hook install post_install SETCAP_FAIL=1; then
    fail "setcap-failure: post_install succeeded"
else
    output_has "cap_net_admin grant" install || fail "setcap-failure: failure was not reported"
    output_has "Partial TUN privileges were removed" install ||
        fail "setcap-failure: cleanup was not reported"
    assert_unprivileged install
    pass "setcap failure revokes partial privileges"
fi

note "ownership failure"
setup_case chown
if run_hook chown post_install CHOWN_FAIL=1; then
    fail "chown-failure: post_install succeeded"
else
    output_has "ownership" chown || fail "chown-failure: failure was not reported"
    assert_unprivileged chown
    pass "ownership failure leaves helpers unprivileged"
fi

note "final state verification"
setup_case verify
if run_hook verify post_install CHOWN_NOOP=1; then
    fail "verify-failure: post_install succeeded"
else
    output_has "not root:v2ray-rs" verify || fail "verify-failure: mismatch was not reported"
    assert_unprivileged verify
    pass "ownership verification failure revokes privileges"
fi

note "removal"
if run_hook install pre_remove; then
    [ "$(identity_count group install)" = 1 ] || fail "removal: group identity was lost"
    [ "$(identity_count user install)" = 1 ] || fail "removal: user identity was lost"
    [ ! -s "$work/install/unexpected" ] || fail "removal: an identity deletion was attempted"
    output_has "Retaining" install || fail "removal: retention was not reported"
    pass "removal keeps system identities"
else
    fail "removal: pre_remove failed"
fi

note "reinstall"
if run_hook install post_install; then
    assert_helpers install
    [ "$(call_count groupadd install)" = 1 ] || fail "reinstall: groupadd ran again"
    [ "$(call_count useradd install)" = 1 ] || fail "reinstall: useradd ran again"
    [ "$(identity_count group install)" = 1 ] || fail "reinstall: group identity was duplicated"
    [ "$(identity_count user install)" = 1 ] || fail "reinstall: user identity was duplicated"
    pass "reinstall reuses existing identities"
else
    fail "reinstall: post_install failed: $(tail -n 4 "$work/install/out" | tr '\n' ' ')"
fi

if [ "$failures" -ne 0 ]; then
    printf '\n%d hook check(s) failed\n' "$failures" >&2
    exit 1
fi
printf '\nall hook checks passed\n'
