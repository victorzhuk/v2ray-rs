## Context

v2ray-rs generates only local SOCKS5/HTTP inbounds; nothing routes whole-system
traffic through the proxy. All backend config is built with `serde_json::json!`
macros, and `AppSettings` is the single input surface — so emitting a `tun` inbound
is mechanically trivial. The repo has **zero** privilege infrastructure (no
`pkexec`/`setcap`/`ip`/`iptables`), and the backend is spawned as the desktop user
and stopped via SIGTERM→SIGKILL with a profile-scoped PID-ownership file.

Upstream reality diverges sharply by backend:
- **sing-box** `tun` + `auto_route: true` programs *and tears down* its own
  `ip route`/`ip rule` tables inside its (capability-holding) process.
- **xray** `tun` creates the device but does **not** configure routes on Linux
  (`autoSystemRoutingTable` is Windows-only).
- **v2ray-core** has no native `tun` inbound.

Creating a TUN device and routing tables requires `CAP_NET_ADMIN`, which the
unprivileged GUI process does not have.

## Goals / Non-Goals

**Goals:**
- System-wide transparent proxying via **sing-box** and **xray**.
- A **one-time** privilege grant, after which connect/disconnect work with the
  existing spawn-as-user → SIGTERM-stop model unchanged.
- **Reliable teardown**: no leftover TUN device or stale routes after a clean stop,
  a crash, or a SIGKILL.
- A **moderate** TUN settings page consistent with the existing DNS page.

**Non-Goals:**
- v2ray TUN (no upstream support).
- Windows/macOS routing automation; per-app/per-UID split routing.
- An iptables `:53` DNS-redirect for xray (DNS is routed *through* the TUN for the
  MVP; a redirect is a scoped follow-up if leaks appear).
- A long-running root helper daemon.

## Decisions

**1. Backend-specific routing: sing-box self-routes, xray uses a route helper.**
sing-box `auto_route` removes all routing code from our app. xray needs a privileged
actor to run the equivalent of `ip addr add` + `ip route add 0.0.0.0/1|128.0.0.0/1`.
*Alternative considered:* sing-box only (simplest) — rejected per product decision to
support both backends.

**2. Privilege via one-time `setcap`, elevated by `pkexec`.**
A user-triggered action runs `setcap 'cap_net_admin,cap_net_bind_service,cap_net_raw+ep'`
on the backend binary (and `cap_net_admin+ep` on the route helper). Thereafter the
backend runs as the user *with capabilities*, so the existing process spawn/stop path
is untouched. *Alternatives:* per-connect `pkexec` (rejected — the backend would run as
root, so the user-level GUI can't SIGTERM it and the whole process layer would need a
helper/`pkexec kill`, plus a prompt every connect); a root helper daemon (rejected —
packaging/IPC/lifecycle cost far exceeds the MVP).

**3. The xray route helper is a separate, minimal setcap'd binary (`v2ray-rs-netctl`).**
It uses `rtnetlink` to program the interface address + split routes directly with its
own effective capability, and exposes idempotent `xray-up` / `xray-down` / `recover`
subcommands. Keeping it tiny and argument-validated limits the attack surface of a
setcap'd binary. *Alternatives:* shelling `/usr/bin/ip` from the helper with ambient
capabilities (fiddlier, more failure modes); a NOPASSWD polkit policy (a different,
broader privilege model than the chosen setcap approach).

**4. TUN is additive.** The `tun` inbound coexists with the existing SOCKS/HTTP
inbounds; nothing about the current inbound behavior changes.

**5. Teardown is layered.** Stop is SIGTERM-first (lets sing-box tear down its tables
and lets xray close its fd so the kernel auto-removes device-scoped routes), then
SIGKILL after the timeout. Orphan cleanup gains the missing SIGKILL fallback, and an
unclean shutdown triggers a `recover` pass on next start.

**6. Moderate UI.** Primary fields (enable, interface, MTU, address) plus an Advanced
expander (stack, strict route, DNS hijack, excluded routes), with backend/capability
gating and an inline grant action.

## Risks / Trade-offs

- **SIGKILL skips sing-box's own route teardown** → stale `ip rule`/tables.
  *Mitigation:* SIGTERM-first stop; `recover --singbox` flushes sing-box's default
  rule/table indices on the next unclean start.
- **`setcap` modifies a system-managed binary** and resets on package upgrade.
  *Mitigation:* re-detect file capabilities before each TUN start and re-prompt;
  document; a future option could setcap an app-owned copy of the binary.
- **xray TUN device-appears race** before the helper runs. *Mitigation:* bounded poll
  on `/sys/class/net/<iface>`; transition to `Error` if it never appears.
- **DNS leaks on xray** (no `:53` redirect in the MVP). *Mitigation:* route DNS through
  the TUN and document; redirect is a scoped follow-up.
- **Wider attack surface** — a setcap'd backend/helper is invokable by any local
  process. *Mitigation:* keep `netctl` minimal and argument-validated; the privilege is
  explicit, informed, and opt-in.

## Migration Plan

- The `[tun]` settings section is new and defaults to **disabled**, so existing
  `settings.toml` files load unchanged (no migration step).
- Packaging must ship the `v2ray-rs-netctl` binary and document the `setcap`
  capability requirement; the app itself drives the grant.
- **Rollback:** disabling TUN restores today's behavior exactly; no persisted data
  shape is removed or repurposed.
