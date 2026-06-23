# Proposal: add-tun-mode

## Why

Today v2ray-rs only generates local SOCKS5/HTTP inbounds, so traffic is proxied
only when an app is explicitly pointed at `127.0.0.1:1080/1081`. There is no way to
route *all* system traffic through the active proxy. TUN mode closes that gap: the
backend creates a virtual network interface and becomes the default route, giving a
full transparent / VPN-style tunnel with no per-app configuration.

## What Changes

- Add a `tun: TunConfig` section to `AppSettings` (persisted to `settings.toml`):
  `enabled`, `interface_name`, `mtu`, `address_v4`, `address_v6`, and the moderate
  advanced knobs `stack`, `strict_route`, `dns_hijack`, `exclude_routes`.
- Emit a `tun` inbound from the **sing-box** and **xray** generators when TUN is
  enabled (additive — TUN coexists with the existing SOCKS/HTTP inbounds):
  - sing-box: a native `tun` inbound with `auto_route: true` (sing-box programs and
    tears down its own routing tables) plus `route.auto_detect_interface: true` for
    loop prevention.
  - xray: a native `tun` protocol inbound with `autoOutboundsInterface: "auto"` for
    loop prevention. xray does **not** configure routes on Linux, so the app drives a
    small privileged route helper to assign the address and add the
    `0.0.0.0/1` + `128.0.0.0/1` split routes.
- **v2ray-core is excluded** — it has no native TUN inbound. With the v2ray backend
  selected, TUN generation is omitted and the UI greys the toggle out.
- Introduce the project's first privilege surface: a **one-time `setcap` via `pkexec`**
  flow that grants `cap_net_admin` (+ `cap_net_bind_service`, `cap_net_raw`) to the
  backend binary, and `cap_net_admin` to the new route helper. The existing
  spawn-as-user → SIGTERM-stop process model is preserved; the backend runs as the
  user *with capabilities*.
- Add a minimal, setcap'd route-helper binary (`v2ray-rs-netctl`, new `crates/netctl`)
  with idempotent `xray-up` / `xray-down` / `recover` subcommands.
- Make TUN teardown reliable: graceful SIGTERM-first stop, a SIGKILL fallback in
  orphan cleanup, and a route-recovery pass after an unclean shutdown so a crash never
  leaves a TUN device or stale routes behind.
- Surface a moderate TUN settings page in the preferences dialog, with backend/cap
  gating, an inline "Grant TUN privileges" action, and a system-wide-routing warning.

## Capabilities

### New Capabilities
- `tun-mode`: TUN inbound generation for sing-box and xray, the capability/privilege
  model (one-time setcap via pkexec, cap detection), loop prevention, the route helper,
  TUN-aware teardown/recovery, and backend gating (v2ray excluded).
- `tun-preferences-ui`: the moderate TUN configuration page in the preferences dialog
  (enable toggle, interface/MTU/address, advanced expander, privilege-grant action,
  warning toast).

### Modified Capabilities
- `app-persistence`: `AppSettings` gains a `tun: TunConfig` field with a backward-
  compatible default and validation.
- `config-generator`: the sing-box and xray (v2ray-family) generators emit a `tun`
  inbound when TUN is enabled; loop-prevention knobs are set per backend.
- `process-lifecycle`: connection start/stop becomes TUN-aware (xray wait-for-device +
  route-helper up/down, graceful-first stop), and orphan cleanup gains a SIGKILL
  fallback plus route recovery.

## Impact

- **New code**: `crates/core/src/models/tun.rs`, `crates/netctl/` (route-helper crate),
  `crates/ui/src/preferences/tun.rs`, a privilege module (cap detect + pkexec grant).
- **Modified models**: `crates/core/src/models/settings.rs`, `models/mod.rs`,
  `models/validation.rs`.
- **Modified generators**: `crates/core/src/config/v2ray.rs` (xray inherits),
  `crates/core/src/config/singbox.rs`, `crates/core/src/config/writer.rs`.
- **Modified process layer**: `crates/process/src/manager.rs`, `crates/process/src/pid.rs`.
- **Modified UI**: `crates/ui/src/preferences/mod.rs`, `crates/ui/src/app.rs`.
- **Dependencies**: add `caps` (file-capability detection) and `rtnetlink` + `tokio`
  (route helper). New workspace member `crates/netctl`.
- **Packaging**: `pkg/archlinux/PKGBUILD` ships the `v2ray-rs-netctl` binary and
  documents the `setcap` capability requirement.
- **Security**: TUN routes all system traffic and requires elevated capabilities on the
  backend + helper. Privilege is opt-in and one-time; the helper is intentionally tiny
  and argument-validated. setcap on a system-managed binary resets on package upgrade —
  the app re-detects and re-prompts. No protocol logic is added; backends still do all
  protocol work.
