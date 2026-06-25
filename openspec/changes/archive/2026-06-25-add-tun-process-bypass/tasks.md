# Tasks: add-tun-process-bypass

## 1. Route helper per-UID rule

- [x] 1.1 In `crates/netctl/src/net.rs`, add `const RULE_PREF_BYPASS_UID: u32 = 8998;` and import `RuleUidRange` alongside the existing rule types.
- [x] 1.2 Add `add_bypass_uid_rule(handle, family, uid)` installing a `uidrange <uid>-<uid> → main` rule at pref 8998 (mirroring `add_rule`, pushing `RuleAttribute::UidRange(RuleUidRange { start: uid, end: uid })`).
- [x] 1.3 Extend `xray_up` with `bypass_uid: Option<u32>`; when `Some`, install the rule per address family alongside `add_xray_rules`.
- [x] 1.4 Add `RULE_PREF_BYPASS_UID` to `is_xray_rule` so `del_xray_rules` (down + recover) removes it.
- [x] 1.5 In `crates/netctl/src/main.rs`, add a validated `--bypass-uid <u32>` argument to `xray-up` and pass it through.
- [x] 1.6 Extend the `privileged-tests` netns suite: assert the pref-8998 uidrange rule is installed by `xray-up --bypass-uid` and removed by `xray-down`/`recover`.

## 2. Bypass user resolution + process wiring

- [x] 2.1 Add `bypass_uid: Option<u32>` to `TunRuntime` in `crates/process/src/tun.rs`; append `--bypass-uid <uid>` to the `xray-up` invocation when set.
- [x] 2.2 Resolve the `v2ray-rs-bypass` UID via `nix::unistd::User::from_name` where `TunRuntime` is built (`crates/ui/src/connection.rs`); add the `user` feature to the workspace `nix` dependency. Absent user ⇒ `None`, connection proceeds.
- [x] 2.3 Add a `run_path()` resolver in `crates/process/src/tun.rs` mirroring `helper_path()` (sibling of exe, else `$PATH`) plus a `RUN_BIN` constant.

## 3. The `v2ray-rs-run` wrapper crate

- [x] 3.1 Create `crates/run` (`[[bin]] v2ray-rs-run`); add `libc` to `[workspace.dependencies]` and depend on it.
- [x] 3.2 Implement: resolve the `v2ray-rs-bypass` UID/GID, `setresgid`/`setresuid` to it (fail closed if the drop fails), then `execvp` the remaining argv. No shell, no env passthrough beyond argv.
- [x] 3.3 Unit test the privilege-drop path (asserts it never execs while still root / aborts on a failed drop).

## 4. Grant flow

- [x] 4.1 In `crates/process/src/privilege.rs`, extend the one-shot `pkexec` grant so that, when the `v2ray-rs-run` binary is present, the same elevation sets root ownership and the setuid bit on it (alongside the existing `setcap` calls).
- [x] 4.2 Tests: grant command construction includes the wrapper step when the binary exists and omits it otherwise (no real elevation in tests).

## 5. UI Run-with-bypass action

- [x] 5.1 In `crates/ui/src/preferences/tun.rs`, add a *Run with bypass* group (xray backend) with a command entry and a launch button that spawns the command via `run_path()`.
- [x] 5.2 Disable the action with a note when the wrapper binary cannot be located; the group is unnecessary for sing-box (native process exclusion).

## 6. Packaging

- [x] 6.1 `pkg/archlinux/PKGBUILD`: add `-p v2ray-rs-run` to the build step and `install -Dm4755 target/release/v2ray-rs-run "$pkgdir/usr/bin/v2ray-rs-run"`.
- [x] 6.2 `pkg/archlinux/v2ray-rs.install`: in `post_install`/`post_upgrade`, create the `v2ray-rs-bypass` system user (`useradd --system --no-create-home --shell /usr/bin/nologin`, idempotent) and `chmod u+s /usr/bin/v2ray-rs-run` (in case pacman stripped it); add a `pre_remove` that runs `userdel v2ray-rs-bypass`.

## 7. Verification & docs

- [x] 7.1 `cargo test --workspace` green.
- [ ] 7.2 Manual (xray TUN on): `v2ray-rs-run curl ifconfig.me` shows the real IP while a plain `curl` shows the proxy IP; `cloudflared tunnel` launched via Run-with-bypass connects.
- [x] 7.3 Update `CLAUDE.md` (the `crates/run` wrapper, the `v2ray-rs-bypass` user, the netctl `--bypass-uid` rule) and `CHANGELOG.md` (Added entry).
