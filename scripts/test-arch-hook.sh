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
[ "${GETCAP_FAIL:-0}" = 1 ] && exit 1
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
# A successful grant that leaves the stored capset untouched lets a fixture
# present wrong flags or extra capabilities through the grant path.
if [ "${SETCAP_NOOP:-0}" = 1 ]; then
    printf 'setcap %s %s\n' "$capabilities" "$*" >>"$CALL_LOG"
    exit 0
fi
for file in "$@"; do
    printf '%s %s\n' "$file" "$capabilities" >"$file.caps"
done
printf 'setcap %s %s\n' "$capabilities" "$*" >>"$CALL_LOG"
EOF
chmod 0755 "$stub"/*

setup_case() {
    local name=$1 root="$work/$1" bin

    mkdir -p "$root/usr/bin"
    for bin in v2ray-rs-netctl v2ray-rs-run v2ray-rs-antidpi; do
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

# Seeds the capset the stub getcap reports for one helper. The first line is
# the path prefix the real getcap prints before the capset.
seed_caps() {
    local name=$1 bin=$2 spec=$3
    printf '%s %s\n' "$work/$name/usr/bin/$bin" "$spec" >"$work/$name/usr/bin/$bin.caps"
}

# Pre-sets the ownership readback so a case can keep seeded caps alive through
# the ownership step (CHOWN_NOOP skips the stub chown that clears caps).
seed_owner() {
    printf 'root:v2ray-rs\n' >"$work/$1/usr/bin/$2.owner"
}

run_hook() {
    local name=$1 function=$2
    shift 2
    env PATH="$stub:$PATH" \
        IDENTITY_LEDGER="$work/$name/identities" \
        CALL_LOG="$work/$name/calls" \
        UNEXPECTED_LOG="$work/$name/unexpected" \
        CHOWN_FAIL=0 CHOWN_NOOP=0 SETCAP_FAIL=0 SETCAP_NOOP=0 GETCAP_FAIL=0 "$@" \
        bash -c '. "$1"; "$2"' _ "$work/$name/hook.sh" "$function" \
        >"$work/$name/out" 2>&1
}

mode_of() { "$real_stat" -c %a "$1"; }
owner_of() {
    if [ -f "$1.owner" ]; then cat "$1.owner"; else printf 'root:root\n'; fi
}
# Exact stored capset line, or empty when the helper carries no capabilities.
caps_of() { cat "$1.caps" 2>/dev/null || true; }
identity_count() { grep -c "^$1:" "$work/$2/identities" 2>/dev/null || true; }
call_count() { grep -c "^$1" "$work/$2/calls" 2>/dev/null || true; }
output_has() { grep -q "$1" "$work/$2/out"; }

assert_helpers() {
    local name=$1
    local netctl="$work/$1/usr/bin/v2ray-rs-netctl"
    local run="$work/$1/usr/bin/v2ray-rs-run"
    local antidpi="$work/$1/usr/bin/v2ray-rs-antidpi"

    [ "$(mode_of "$netctl")" = 750 ] || fail "$name: netctl mode is $(mode_of "$netctl"), want 750"
    [ "$(mode_of "$run")" = 4750 ] || fail "$name: wrapper mode is $(mode_of "$run"), want 4750"
    [ "$(mode_of "$antidpi")" = 750 ] || fail "$name: anti-DPI mode is $(mode_of "$antidpi"), want 750"
    [ "$(owner_of "$netctl")" = root:v2ray-rs ] ||
        fail "$name: netctl owner is $(owner_of "$netctl"), want root:v2ray-rs"
    [ "$(owner_of "$run")" = root:v2ray-rs ] ||
        fail "$name: wrapper owner is $(owner_of "$run"), want root:v2ray-rs"
    [ "$(owner_of "$antidpi")" = root:v2ray-rs ] ||
        fail "$name: anti-DPI owner is $(owner_of "$antidpi"), want root:v2ray-rs"
    [ "$(caps_of "$netctl")" = "$netctl cap_net_admin+ep" ] ||
        fail "$name: netctl capset is '$(caps_of "$netctl")', want '$netctl cap_net_admin+ep'"
    [ "$(caps_of "$antidpi")" = "$antidpi cap_net_admin,cap_net_raw+ep" ] ||
        fail "$name: anti-DPI capset is '$(caps_of "$antidpi")', want the exact cap_net_admin,cap_net_raw=ep set"
}

assert_unprivileged() {
    local name=$1
    local netctl="$work/$1/usr/bin/v2ray-rs-netctl"
    local run="$work/$1/usr/bin/v2ray-rs-run"
    local antidpi="$work/$1/usr/bin/v2ray-rs-antidpi"

    [ "$(mode_of "$netctl")" = 750 ] || fail "$name: netctl mode is $(mode_of "$netctl"), want 750"
    [ "$(mode_of "$run")" = 750 ] || fail "$name: wrapper mode is $(mode_of "$run"), want 750"
    [ "$(mode_of "$antidpi")" = 750 ] || fail "$name: anti-DPI mode is $(mode_of "$antidpi"), want 750"
    for bin in "$netctl" "$antidpi"; do
        [ -z "$(caps_of "$bin")" ] || fail "$name: $bin kept capabilities: $(caps_of "$bin")"
    done
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
    pass "install provisions restricted helpers with exact capsets"
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
    output_has "Privilege cleanup verified" install ||
        fail "setcap-failure: verified cleanup was not reported"
    assert_unprivileged install
    pass "setcap failure reports the grant and verifies clean helpers"
fi

note "wrong flags"
setup_case wrongflags
seed_caps wrongflags v2ray-rs-netctl 'cap_net_admin=eip'
seed_owner wrongflags v2ray-rs-netctl
seed_owner wrongflags v2ray-rs-run
seed_owner wrongflags v2ray-rs-antidpi
if run_hook wrongflags post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    fail "wrong-flags: post_install accepted cap_net_admin=eip"
else
    output_has "cap_net_admin verification" wrongflags ||
        fail "wrong-flags: the flag mismatch was not reported"
    output_has "Privilege cleanup verified" wrongflags ||
        fail "wrong-flags: cleanup did not verify"
    assert_unprivileged wrongflags
    pass "wrong flags are rejected and revoked"
fi

note "extra capabilities"
setup_case extracaps
seed_caps extracaps v2ray-rs-netctl 'cap_net_admin+ep cap_chown+ep'
seed_owner extracaps v2ray-rs-netctl
seed_owner extracaps v2ray-rs-run
seed_owner extracaps v2ray-rs-antidpi
if run_hook extracaps post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    fail "extra-caps: post_install accepted cap_chown beyond the route set"
else
    output_has "cap_net_admin verification" extracaps ||
        fail "extra-caps: the extra capability was not reported"
    assert_unprivileged extracaps
    pass "extra capabilities are rejected and revoked"
fi

note "unreadable probe"
setup_case probe
if run_hook probe post_install GETCAP_FAIL=1; then
    fail "unreadable-probe: post_install succeeded without a readable capset"
else
    output_has "cap_net_admin verification" probe ||
        fail "unreadable-probe: the unreadable probe was not reported"
    output_has "Revocation could not be verified for: route helper capability and anti-DPI helper capability" probe ||
        fail "unreadable-probe: unverified revocation was not reported"
    assert_unprivileged probe
    pass "unreadable probe fails the grant and the revocation"
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
    assert_unprivileged install
    pass "removal revokes privileges and keeps system identities"
else
    fail "removal: pre_remove failed: $(tail -n 4 "$work/install/out" | tr '\n' ' ')"
fi

note "removal with unresolved revocation"
if run_hook install pre_remove GETCAP_FAIL=1; then
    fail "removal-unresolved: pre_remove returned success with an unreadable probe"
else
    output_has "Unresolved: helper privilege cleanup did not verify" install ||
        fail "removal-unresolved: the unresolved cleanup was not reported"
    [ "$(identity_count group install)" = 1 ] || fail "removal-unresolved: group identity was lost"
    [ "$(identity_count user install)" = 1 ] || fail "removal-unresolved: user identity was lost"
    [ ! -s "$work/install/unexpected" ] || fail "removal-unresolved: an identity deletion was attempted"
    pass "unresolved revocation fails pre_remove and keeps identities"
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
