# Proposal: add-tun-process-bypass

## Why

`add-tun-exclude-routing` lets users exclude *destinations* (CIDR/domain) from the
xray TUN and excludes *processes* natively on sing-box. But xray cannot match TUN
traffic by process, so a tool with unpredictable destinations (e.g. a freshly
launched `cloudflared`) still has no per-process escape on xray. This change adds
per-process bypass for xray for tools the user launches through the app, using a
dedicated UID and the existing fwmark-255 bypass plumbing — without expanding the
privilege of the world-callable route helper.

## What Changes

- Add a dedicated unprivileged system user `v2ray-rs-bypass`. The application
  resolves its UID (`getpwnam`) and passes it to the route helper.
- The route helper (`v2ray-rs-netctl`) gains a per-UID policy rule: on xray TUN
  up it installs an `ip rule uidrange <uid> → main` rule (priority ahead of the
  unmarked-to-main rule), per address family, removing it on down and recovery.
  This stays pure rtnetlink (`RuleAttribute::UidRange`) — no new dependency.
- Add a minimal setuid-root wrapper binary `v2ray-rs-run` (new `crates/run`) that
  drops to the `v2ray-rs-bypass` UID/GID then `execvp`s the requested command, so
  the command and its children carry the bypass UID and match the policy rule.
  Their DNS bypasses too, because bypass-UID traffic never enters the tunnel.
- The TUN preferences page gains a *Run with bypass* action (xray) that launches a
  user-supplied command through the wrapper.
- The one-time `pkexec` grant additionally ensures the wrapper is root-owned with
  the setuid bit set; packaging installs the user and the setuid binary.

Catching already-running, externally-launched tools on xray is explicitly out of
scope — it would require a privileged `/proc` reconciler and `cap_sys_admin` on a
helper any local process can call. sing-box already covers already-running tools
by name.

## Capabilities

### New Capabilities
- `tun-process-bypass`: the dedicated bypass user, the route helper's per-UID
  policy rule, the setuid `v2ray-rs-run` launcher, the Run-with-bypass UI action,
  and the scope boundary (app-launched xray tools only).

### Modified Capabilities
- `tun-mode`: the one-time privilege grant also ensures the `v2ray-rs-run` wrapper
  is root-owned and setuid.
- `process-lifecycle`: xray TUN start resolves the bypass UID and passes
  `--bypass-uid` to the route helper.

## Impact

- **New crate**: `crates/run` (`v2ray-rs-run` bin; `libc` for setuid/setgid/
  execvp). Auto-included via the `crates/*` workspace glob; add `libc` to
  `[workspace.dependencies]`.
- **Modified netctl**: `crates/netctl/src/{net,main}.rs` — `RULE_PREF_BYPASS_UID`,
  a uidrange rule via `RuleAttribute::UidRange(RuleUidRange { start, end })`, a
  `--bypass-uid` argument, and the new priority added to the rule-teardown filter.
- **Modified process layer**: `crates/process/src/{tun,manager,privilege}.rs` —
  `TunRuntime.bypass_uid`, pass-through to `xray-up`, the wrapper setuid step in
  `grant`, and a `run_path()` resolver mirroring `helper_path()`.
- **Modified UI**: `crates/ui/src/preferences/tun.rs` (Run-with-bypass action),
  `crates/ui/src/connection.rs` (resolve the bypass UID via nix `User::from_name`;
  add the `user` nix feature).
- **Packaging**: `pkg/archlinux/PKGBUILD` builds `-p v2ray-rs-run`, installs it
  mode 4755, and the `.install` hook creates/removes the `v2ray-rs-bypass` user.
- **Security**: the only new privileged surface is the tiny `v2ray-rs-run` wrapper,
  which drops privilege before exec; the route helper stays `cap_net_admin`-only
  and its new primitive is a fixed per-UID rule, not arbitrary-PID marking.
- **Sequencing**: applies after `add-tun-exclude-routing` (reuses its TUN
  exclusion UI section and `TunConfig` model).
