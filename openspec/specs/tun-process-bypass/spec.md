## Purpose

Let selected process traffic bypass the xray TUN tunnel via a dedicated unprivileged user and setuid wrapper.

## Requirements

### Requirement: Dedicated bypass user and per-UID policy rule
The system SHALL provide a dedicated unprivileged system user `v2ray-rs-bypass`
whose traffic bypasses the xray TUN tunnel. When xray TUN is brought up and the
bypass user exists, the privileged route helper SHALL install a policy rule that
diverts traffic owned by that user's UID to the main routing table, at a priority
evaluated ahead of the unmarked-to-main rule, for each address family in use; the
rule SHALL be removed on TUN down and on recovery. The UID SHALL be resolved by
the application (`getpwnam`) and passed to the helper as an integer argument; the
helper SHALL remain dependency-light (pure rtnetlink, `RuleAttribute::UidRange`)
and SHALL NOT resolve users itself.

#### Scenario: Bypass rule installed on xray TUN up
- **WHEN** xray TUN is brought up, the bypass user exists, and its UID is passed to the route helper
- **THEN** the helper SHALL install a `uidrange` policy rule for that UID pointing at the main table, at a priority ahead of the unmarked-to-main rule, for IPv4 and (when an IPv6 address is configured) IPv6

#### Scenario: Bypass rule removed on down and recover
- **WHEN** the route helper tears down or recovers the xray TUN
- **THEN** the per-UID bypass rule SHALL be removed alongside the other helper-installed policy rules, leaving no leftover rule

#### Scenario: Bypass user absent
- **WHEN** the `v2ray-rs-bypass` user does not exist (e.g. an unpackaged or dev install)
- **THEN** no bypass rule SHALL be installed and the TUN connection SHALL proceed normally without error

### Requirement: Launch tools outside the tunnel via a setuid wrapper
The system SHALL provide a minimal setuid-root wrapper binary `v2ray-rs-run` that
drops its real, effective, and saved UID and GID to the `v2ray-rs-bypass` user
before executing the requested command, so the command and its child processes
carry the bypass UID and match the per-UID policy rule. The wrapper SHALL be
minimal and SHALL NOT pass any elevated privilege to the target command. It SHALL
be located the same way as the route helper — a sibling of the running executable,
falling back to `$PATH`.

#### Scenario: Wrapper drops privilege then executes
- **WHEN** `v2ray-rs-run <command>` is invoked
- **THEN** the wrapper SHALL set its UID and GID to the `v2ray-rs-bypass` user and only then `execvp` the command, so the command never runs as root

#### Scenario: Children inherit the bypass identity
- **WHEN** a command launched through the wrapper spawns child processes
- **THEN** those children SHALL inherit the bypass UID and their traffic SHALL also match the per-UID policy rule

### Requirement: Run-with-bypass UI action
The TUN preferences page SHALL offer an action to launch a user-supplied command
through the `v2ray-rs-run` wrapper. The action SHALL be available for the xray
backend, where per-process bypass requires the wrapper, and SHALL be disabled when
the wrapper binary is not present. For sing-box the action is unnecessary because
sing-box excludes processes by name natively.

#### Scenario: Launch a command outside the tunnel (xray)
- **WHEN** the active backend is xray, TUN is connected, and the user enters a command in the Run-with-bypass action
- **THEN** the system SHALL launch the command through `v2ray-rs-run` so its traffic bypasses the tunnel

#### Scenario: Action disabled without the wrapper
- **WHEN** the `v2ray-rs-run` binary cannot be located
- **THEN** the Run-with-bypass action SHALL be insensitive with a note rather than failing on use

### Requirement: Scope of xray per-process bypass
Per-process bypass on xray SHALL apply only to tools launched through the
`v2ray-rs-run` wrapper. The system SHALL NOT move already-running processes
between cgroups, SHALL NOT require `cap_sys_admin`, and SHALL NOT add a `/proc`
reconciler for xray. Exclusion of already-running tools by name SHALL remain a
sing-box capability.

#### Scenario: Already-running xray tool is not auto-bypassed
- **WHEN** a tool is already running under the user's normal UID (started from a terminal or a systemd unit) while xray TUN is active
- **THEN** its traffic SHALL continue through the tunnel, and the documented way to bypass it on xray SHALL be to relaunch it via Run-with-bypass or to use the sing-box backend

### Requirement: Setuid wrapper verifies identity drop and sanitizes environment
Before executing the target command, `v2ray-rs-run` SHALL verify that real, effective, and saved UID and GID match the `v2ray-rs-bypass` account and that supplementary groups are cleared. It SHALL clear the inherited environment except for an explicit allowlist of safe session variables.

#### Scenario: Wrapper verifies UID and GID drop
- **WHEN** `v2ray-rs-run` drops privileges to `v2ray-rs-bypass`
- **THEN** it SHALL read back real, effective, and saved UID and GID and abort without calling `execvp` if any ID differs from `v2ray-rs-bypass`

#### Scenario: Untrusted environment variables stripped
- **WHEN** `v2ray-rs-run` is invoked with `PYTHONPATH`, `LD_AUDIT`, `GIO_EXTRA_MODULES`, `GTK_MODULES`, or `BASH_ENV` set alongside `PATH` and `WAYLAND_DISPLAY`
- **THEN** the executed child SHALL receive only allowlisted variables (`PATH`, `HOME`, `USER`, `LOGNAME`, `TERM`, `LANG`, `LC_*`, `DISPLAY`, `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`, `XAUTHORITY`) with `HOME`, `USER`, and `LOGNAME` consistent with the dropped execution context

### Requirement: Privileged setup provisions bypass user and archive modes stay unprivileged
One-time `pkexec` privilege grant and distribution post-install hooks SHALL ensure the `v2ray-rs-bypass` system user exists when `v2ray-rs-run` is present. Package payloads and release archives SHALL ship `v2ray-rs-run` with mode `0755` and apply `root:v2ray-rs` `4750` only in root install hooks or `pkexec` grant.

#### Scenario: pkexec grant creates missing bypass user
- **WHEN** a user runs "Grant TUN privileges" on a host where `v2ray-rs-run` is present but the `v2ray-rs-bypass` system user does not yet exist
- **THEN** the elevated grant script SHALL create the unprivileged `v2ray-rs-bypass` system account (`nologin` shell, nonexistent home) before verifying installation state

#### Scenario: Arch package payload contains no setuid bit
- **WHEN** `pkg/archlinux/PKGBUILD` or `pkg/archlinux/bin/PKGBUILD` builds a package archive
- **THEN** `/usr/bin/v2ray-rs-run` in the archive payload SHALL have mode `0755`, and `post_install` / `post_upgrade` SHALL set `root:v2ray-rs` and mode `4750` after `systemd-sysusers` creates the group and bypass user

### Requirement: Setuid wrapper closes inherited file descriptors before execution
Before invoking `execvp`, `v2ray-rs-run` SHALL close all inherited open file descriptors greater than `2` (`stderr`) so non-`FD_CLOEXEC` descriptors held by the caller are never inherited by the `v2ray-rs-bypass` target process.

#### Scenario: Open caller descriptor >= 3 is closed across wrapper execution
- **WHEN** a caller opens a file or socket descriptor `>= 3` without `FD_CLOEXEC` and spawns a command via `v2ray-rs-run`
- **THEN** the executed target process SHALL observe only standard descriptors `0`, `1`, and `2` open from the caller
