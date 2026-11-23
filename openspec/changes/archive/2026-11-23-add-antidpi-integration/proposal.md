# Proposal: add-antidpi-integration

## Why

The anti-DPI helper `v2ray-rs-antidpi` (crate `crates/antidpi`) already exists and is
sandbox-verified (`docs/antidpi-registry.md` §8), but it is a standalone CLI: nothing
in the app starts it, grants its capabilities, configures it, or ships it. On
DPI-active ISPs (TSPU throttling/RSTs) a user would have to run it by hand with the
right privileges, in the right order relative to the backend, and tear it down
manually — error-prone and undiscoverable. The app already owns a proven
privileged-helper pattern (the `netctl` route helper: one-time `pkexec setcap`
grant, group-restricted helpers, bounded `run_helper` invocation with log capture),
so wiring the engine into the existing connection lifecycle closes the gap without
adding any protocol logic — the engine keeps handling packet surgery; the app only
orchestrates.

## What Changes

- Add an `[antidpi]` section to `AppSettings`: `enabled`, `queue`, `strategies`
  (list of strategy chain strings; empty = the helper's built-in default chains),
  and an optional hostlist path. Persisted through an `AntiDpiConfigWire`
  forward-compat wire struct with `validate()`, mirroring `TunConfig`/`TunConfigWire`.
- Treat anti-DPI settings as restart-relevant: `RuntimeConfigSnapshot` captures and
  restores them, so toggling the engine, queue, or strategies while connected raises
  the restart-required banner and "Apply & Restart" restarts the session with the
  new engine arguments.
- Introduce an `AntiDpiRuntime` in the process crate: helper resolution via the
  existing `pick_bin` pattern (`ANTIDPI_BIN` const), a one-shot `install-rules`
  before the backend spawn, the long-lived engine child (`run --no-fw`) spawned
  after route setup (`xray_up`) or directly after the backend spawn when TUN is
  off, and SIGTERM-first engine termination plus a one-shot `remove-rules`
  safeguard on stop — all logged through the existing helper log stream.
- Extend the privilege grant to cover the antidpi binary with
  `cap_net_admin,cap_net_raw+ep`, `root:v2ray-rs` `0750` — same single `pkexec`
  elevation, extended plan and elevated script.
- Add an Anti-DPI preferences page mirroring `preferences/tun.rs`: enable switch,
  queue, a shape-validated strategy editor, and a hostlist path. The page states
  what the engine actually covers (TLS and QUIC on port 443) and points at the
  shared "Grant TUN privileges" elevation on the TUN page, whose capability row
  also reports a missing anti-DPI capability; the page itself offers no inline
  grant button.
- Install the firewall rules in an owned nftables table through the privileged
  helper's own netlink client, never by executing a system firewall command, and
  track them in a write-ahead ownership journal keyed by controller, boot,
  network namespace, and session. The one-shot `install-rules` validates every
  strategy and the hostlist before any mutation, so a malformed chain fails the
  connection before the backend spawns. The engine reports readiness only after
  its NFQUEUE bind, and a pre-ready exit is a startup failure rather than a
  silent "connected".
- Ship the binary through every existing distribution channel: PKGBUILD build +
  install lines, the Arch install-hook setcap loop, the bin PKGBUILD, `make dist`
  + `stage-dist.sh`, `install.sh`, and the Debian metadata.

## Capabilities

### New Capabilities
- `antidpi-integration`: the engine lifecycle wiring in the process layer — helper
  resolution and access classification, the install-rules/engine-spawn sequencing
  relative to the backend and TUN, capability gating, session-scoped teardown, and
  failure visibility.
- `antidpi-preferences-ui`: the preferences page — enable toggle, queue, strategy
  list, hostlist path, effect wording, and the grant action.

### Modified Capabilities
- `app-persistence`: `AppSettings` gains an `antidpi` field with a
  backward-compatible default and validation.
- `process-lifecycle`: the runtime snapshot treats anti-DPI settings as
  restart-relevant.
- `release-packaging`: packages build and install the antidpi helper; the Arch hook
  provisions its privileges; the dist tarball and installers carry it; the Makefile
  covers the ninth crate.

## Impact

- **New code**: `crates/core/src/models/antidpi.rs`, `crates/process/src/antidpi.rs`,
  `crates/ui/src/preferences/antidpi.rs`.
- **Modified models**: `crates/core/src/models/settings.rs`, `models/mod.rs`,
  `models/validation.rs`, `crates/core/src/runtime_snapshot.rs`.
- **Modified process layer**: `crates/process/src/manager.rs`,
  `crates/process/src/session/mod.rs`, `crates/process/src/tun.rs` (binary
  resolution reuse), `crates/process/src/privilege.rs`.
- **Modified UI**: `crates/ui/src/preferences/mod.rs`, i18n catalogs
  (`locale/en_US`, `locale/ru_RU`, `v2ray-rs.pot`).
- **Docs**: `AGENTS.md` (nine crates), `docs/RELEASE.md`, `CHANGELOG.md`.
- **Packaging**: `pkg/archlinux/PKGBUILD`, `pkg/archlinux/v2ray-rs.install`,
  `pkg/archlinux/bin/PKGBUILD`, `Makefile`, `scripts/stage-dist.sh`,
  `scripts/install.sh`, `crates/ui/Cargo.toml` (deb metadata).
- **Dependencies**: none new — `crates/antidpi` is already a workspace member
  (`members = ["crates/*"]`); the dependency direction stays one-way (process →
  antidpi only as an external binary, never as a crate dependency).
- **Security**: adds a second setcap'd binary holding `cap_net_admin` +
  `cap_net_raw`. Same mitigations as `netctl`: restricted to the dedicated
  `v2ray-rs` group with `root:v2ray-rs` `0750`, opt-in grant through a verified
  `pkexec` script that re-validates every mutation target, and `nosuid` preflight.
  The firewall writes happen inside the fixed-purpose helper over netlink, so no
  system firewall command inherits or receives privileges. The engine modifies
  only this host's treatment of the early packets of TLS and QUIC flows to port
  443; it adds no protocol logic, talks to no server, and is disabled by default.
  Rules are removed only by the session that owns them, verified through the
  journal; an unowned table is reported, never deleted.
