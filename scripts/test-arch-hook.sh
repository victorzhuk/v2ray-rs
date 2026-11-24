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
[ "${GETCAP_WARN:-0}" = 1 ] && printf 'getcap: diagnostic noise\n' >&2
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
        CHOWN_FAIL=0 CHOWN_NOOP=0 SETCAP_FAIL=0 SETCAP_NOOP=0 GETCAP_FAIL=0 GETCAP_WARN=0 "$@" \
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

note "correct multi-name clause"
setup_case multiname
seed_caps multiname v2ray-rs-netctl 'cap_net_admin=ep'
seed_caps multiname v2ray-rs-antidpi 'cap_net_admin,cap_net_raw=ep'
seed_owner multiname v2ray-rs-netctl
seed_owner multiname v2ray-rs-run
seed_owner multiname v2ray-rs-antidpi
if run_hook multiname post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    [ "$(caps_of "$work/multiname/usr/bin/v2ray-rs-antidpi")" = \
        "$work/multiname/usr/bin/v2ray-rs-antidpi cap_net_admin,cap_net_raw=ep" ] ||
        fail "multi-name: the seeded readback was mutated"
    output_has "usermod -aG v2ray-rs" multiname ||
        fail "multi-name: success guidance missing"
    pass "a comma-joined readback clause normalizes to the exact pair set"
else
    fail "multi-name: post_install rejected 'cap_net_admin,cap_net_raw=ep'"
fi

note "mixed flags in both orders"
setup_case mixedorders
seed_caps mixedorders v2ray-rs-netctl 'cap_net_admin=pe'
seed_caps mixedorders v2ray-rs-antidpi 'cap_net_raw=ep cap_net_admin=ep'
seed_owner mixedorders v2ray-rs-netctl
seed_owner mixedorders v2ray-rs-run
seed_owner mixedorders v2ray-rs-antidpi
if run_hook mixedorders post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    output_has "usermod -aG v2ray-rs" mixedorders ||
        fail "mixed-orders: success guidance missing"
    pass "unordered flag letters and reversed clauses normalize correctly"
else
    fail "mixed-orders: post_install rejected a semantically correct capset"
fi

note "mixed per-capability flags"
setup_case mixedflags
seed_caps mixedflags v2ray-rs-netctl 'cap_net_admin=ep'
seed_caps mixedflags v2ray-rs-antidpi 'cap_net_admin=ep cap_net_raw=e'
seed_owner mixedflags v2ray-rs-netctl
seed_owner mixedflags v2ray-rs-run
seed_owner mixedflags v2ray-rs-antidpi
if run_hook mixedflags post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    fail "mixed-flags: post_install accepted cap_net_raw=e beside cap_net_admin=ep"
else
    output_has "anti-DPI capability verification" mixedflags ||
        fail "mixed-flags: the per-capability flag mismatch was not reported"
    output_has "Privilege cleanup verified" mixedflags ||
        fail "mixed-flags: cleanup did not verify"
    assert_unprivileged mixedflags
    pass "mixed per-capability flags are rejected and revoked"
fi

note "mixed per-capability flags, reversed clause order"
setup_case mixedflagsrev
seed_caps mixedflagsrev v2ray-rs-netctl 'cap_net_admin=ep'
seed_caps mixedflagsrev v2ray-rs-antidpi 'cap_net_raw=e cap_net_admin=ep'
seed_owner mixedflagsrev v2ray-rs-netctl
seed_owner mixedflagsrev v2ray-rs-run
seed_owner mixedflagsrev v2ray-rs-antidpi
if run_hook mixedflagsrev post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    fail "mixed-flags-rev: post_install accepted the reversed mixed-flag clause"
else
    output_has "anti-DPI capability verification" mixedflagsrev ||
        fail "mixed-flags-rev: the per-capability flag mismatch was not reported"
    output_has "Privilege cleanup verified" mixedflagsrev ||
        fail "mixed-flags-rev: cleanup did not verify"
    assert_unprivileged mixedflagsrev
    pass "reversed mixed per-capability flags are rejected and revoked"
fi

note "empty clause name"
setup_case emptynames
seed_caps emptynames v2ray-rs-netctl 'cap_net_admin=ep'
seed_caps emptynames v2ray-rs-antidpi 'cap_net_admin,,cap_net_raw=ep'
seed_owner emptynames v2ray-rs-netctl
seed_owner emptynames v2ray-rs-run
seed_owner emptynames v2ray-rs-antidpi
if run_hook emptynames post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    fail "empty-names: post_install accepted 'cap_net_admin,,cap_net_raw=ep'"
else
    output_has "anti-DPI capability verification" emptynames ||
        fail "empty-names: the empty clause name was not reported"
    output_has "Privilege cleanup verified" emptynames ||
        fail "empty-names: cleanup did not verify"
    assert_unprivileged emptynames
    pass "an empty comma-separated name is rejected and revoked"
fi

note "duplicate flag letters"
setup_case dupflags
seed_caps dupflags v2ray-rs-netctl 'cap_net_admin=epe'
seed_caps dupflags v2ray-rs-antidpi 'cap_net_admin=ep cap_net_raw=ep'
seed_owner dupflags v2ray-rs-netctl
seed_owner dupflags v2ray-rs-run
seed_owner dupflags v2ray-rs-antidpi
if run_hook dupflags post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    fail "dup-flags: post_install accepted cap_net_admin=epe"
else
    output_has "cap_net_admin verification" dupflags ||
        fail "dup-flags: the repeated flag letter was not reported"
    output_has "Privilege cleanup verified" dupflags ||
        fail "dup-flags: cleanup did not verify"
    assert_unprivileged dupflags
    pass "repeated flag letters are rejected and revoked"
fi

note "glob metacharacters in a name"
setup_case globname
seed_caps globname v2ray-rs-netctl 'cap_net_admin=ep cap_*=ei'
seed_owner globname v2ray-rs-netctl
seed_owner globname v2ray-rs-run
seed_owner globname v2ray-rs-antidpi
if run_hook globname post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    fail "glob-name: post_install accepted a wildcard capability name"
else
    output_has "cap_net_admin verification" globname ||
        fail "glob-name: the wildcard name was not reported"
    output_has "Privilege cleanup verified" globname ||
        fail "glob-name: cleanup did not verify"
    assert_unprivileged globname
    pass "a glob metacharacter name is rejected without expansion and revoked"
fi

note "unparseable readback"
setup_case garbled
seed_caps garbled v2ray-rs-netctl 'cap_net_admin=ex cap_net_admin=ep'
seed_owner garbled v2ray-rs-netctl
seed_owner garbled v2ray-rs-run
seed_owner garbled v2ray-rs-antidpi
if run_hook garbled post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    fail "garbled: post_install accepted an unknown flag and a duplicate name"
else
    output_has "cap_net_admin verification" garbled ||
        fail "garbled: the unparseable readback was not reported"
    output_has "Privilege cleanup verified" garbled ||
        fail "garbled: cleanup did not verify"
    assert_unprivileged garbled
    pass "an unknown flag and a duplicate name are rejected and revoked"
fi

note "malformed clause"
setup_case malformedclause
seed_caps malformedclause v2ray-rs-netctl 'cap_net_admin'
seed_owner malformedclause v2ray-rs-netctl
seed_owner malformedclause v2ray-rs-run
seed_owner malformedclause v2ray-rs-antidpi
if run_hook malformedclause post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    fail "malformed-clause: post_install accepted a clause without flags"
else
    output_has "cap_net_admin verification" malformedclause ||
        fail "malformed-clause: the malformed clause was not reported"
    output_has "Privilege cleanup verified" malformedclause ||
        fail "malformed-clause: cleanup did not verify"
    assert_unprivileged malformedclause
    pass "a clause without flags is rejected and revoked"
fi

note "duplicate capability name"
setup_case dupname
seed_caps dupname v2ray-rs-netctl 'cap_net_admin=ep cap_net_admin=ep'
seed_owner dupname v2ray-rs-netctl
seed_owner dupname v2ray-rs-run
seed_owner dupname v2ray-rs-antidpi
if run_hook dupname post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    fail "dup-name: post_install accepted a duplicated capability"
else
    output_has "cap_net_admin verification" dupname ||
        fail "dup-name: the duplicated capability was not reported"
    output_has "Privilege cleanup verified" dupname ||
        fail "dup-name: cleanup did not verify"
    assert_unprivileged dupname
    pass "a duplicate capability name alone is rejected and revoked"
fi

note "unexpected extra output"
setup_case extraout
seed_caps extraout v2ray-rs-netctl 'cap_net_admin=ep'
printf '/elsewhere cap_chown=ep\n' >>"$work/extraout/usr/bin/v2ray-rs-netctl.caps"
seed_owner extraout v2ray-rs-netctl
seed_owner extraout v2ray-rs-run
seed_owner extraout v2ray-rs-antidpi
if run_hook extraout post_install SETCAP_NOOP=1 CHOWN_NOOP=1; then
    fail "extra-out: post_install accepted a second getcap record"
else
    output_has "cap_net_admin verification" extraout ||
        fail "extra-out: the extra record was not reported"
    output_has "Privilege cleanup verified" extraout ||
        fail "extra-out: cleanup did not verify"
    assert_unprivileged extraout
    pass "a correct record plus extra output is rejected and revoked"
fi

note "diagnostic readback"
setup_case diag
if run_hook diag post_install GETCAP_WARN=1; then
    fail "diag: post_install succeeded with a noisy getcap"
else
    output_has "cap_net_admin verification" diag ||
        fail "diag: the diagnostic readback was not reported"
    output_has "Revocation could not be verified for: route helper capability and anti-DPI helper capability" diag ||
        fail "diag: unverified revocation was not reported"
    output_has "Privilege cleanup verified" diag &&
        fail "diag: a diagnostic readback was treated as verified absence"
    assert_unprivileged diag
    pass "a diagnostic readback is unverified, never absence"
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
    output_has "not revoked automatically" install ||
        fail "removal: the backend-grant note was not reported"
    output_has "with its administrator" install ||
        fail "removal: the administrator advice was not reported"
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

note "removal with diagnostic readback"
if run_hook diag pre_remove GETCAP_WARN=1; then
    fail "removal-diag: pre_remove returned success with a noisy getcap"
else
    output_has "Unresolved: helper privilege cleanup did not verify" diag ||
        fail "removal-diag: the unresolved cleanup was not reported"
    output_has "not revoked automatically" diag ||
        fail "removal-diag: the backend-grant note was not reported"
    [ "$(identity_count group diag)" = 1 ] || fail "removal-diag: group identity was lost"
    [ "$(identity_count user diag)" = 1 ] || fail "removal-diag: user identity was lost"
    [ ! -s "$work/diag/unexpected" ] || fail "removal-diag: an identity deletion was attempted"
    pass "a diagnostic readback keeps removal unresolved and identities intact"
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
