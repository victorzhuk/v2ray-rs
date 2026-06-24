# Design: add-tun-process-bypass

## Mechanism: dedicated UID + `ip rule uidrange`

xray has no process matching for TUN traffic, so bypass must happen at the kernel
by *who owns* the packets. Two `cap_net_admin`-only options exist: mark by UID via
`ip rule uidrange` (pure rtnetlink) or mark by GID via `nft`. We chose **UID +
`ip rule uidrange`** because it adds **zero dependency** to the deliberately
dependency-light route helper — it slots in beside the existing policy rules as
another `RuleMessage`. `netlink-packet-route 0.30` exposes
`RuleAttribute::UidRange(RuleUidRange { start, end })` (FRA_UID_RANGE=20),
re-exported from `rtnetlink::packet_route::rule`, so no raw-attribute encoding is
needed.

The rule is `uidrange <uid>-<uid> → table main` at a new priority
`RULE_PREF_BYPASS_UID = 8998`, ahead of the existing `RULE_PREF_MAIN = 9001`
(unmarked → main with the default suppressed) so bypass-UID traffic reaches the
real default instead of falling through to the tunnel table. It is installed per
family in `xray_up` and added to the `is_xray_rule` teardown filter.

## Why this also fixes DNS for free

Bypass-UID traffic is diverted to the main table *before* it can enter the tunnel,
so the bypassed tool's DNS (also owned by the bypass UID) never reaches xray's DNS
hijack — no extra DNS handling is required, unlike the destination-based path in
`add-tun-exclude-routing`.

## The wrapper and privilege boundary

`v2ray-rs-run` is the only new privileged surface. It is setuid-root but its sole
job is `setresuid`/`setresgid` to the `v2ray-rs-bypass` user, then `execvp` — it
never runs the target as root and carries no shell or env passthrough beyond argv.
The UID is resolved by the app (`nix::unistd::User::from_name`, needs the `user`
nix feature) and passed to the helper as an integer; the helper itself never does
user lookups, preserving its dependency-light contract. Crucially, the route
helper's new primitive is a *fixed per-UID rule*, not arbitrary-PID marking, so a
local caller cannot use it to exempt an arbitrary process's traffic.

## Packaging

- Workspace `members = ["crates/*"]` auto-includes `crates/run`; add `libc` to
  `[workspace.dependencies]`.
- PKGBUILD builds `-p v2ray-rs-run` and installs it mode 4755. pacman may strip the
  setuid bit, so the `.install` `post_install`/`post_upgrade` hook re-applies
  `chmod u+s` alongside the existing netctl `setcap`, and creates the
  `v2ray-rs-bypass` system user (`useradd --system --no-create-home --shell
  /usr/bin/nologin`); `pre_remove` deletes it.

## Out of scope

Catching already-running, externally-launched tools on xray is cut. It needs a
`/proc` reconciler moving live PIDs into a cgroup with `cap_sys_admin` on a helper
any local process can call — a local tunnel-escape primitive. sing-box's native
process matching (shipped in `add-tun-exclude-routing`) covers that case; the
documented answer for xray users is to relaunch via Run-with-bypass or switch to
sing-box.
