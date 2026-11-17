# Spec Delta

## ADDED Requirements

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
