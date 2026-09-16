## Why

With xray TUN connected, destinations reachable only through another interface's more-specific route are unreachable through xray's `direct` outbound. With a split-tunnel corporate OpenVPN (`10.0.0.0/8 via tun0`, `never-default`), the VPN's resolver `<vpn-dns-ip>` is captured by the DNS capture rule into the tunnel. xray routes it `[tun-in -> direct]` and sends it out the Wi-Fi uplink. That query times out, or the network's UDP/53 interceptor answers it with a forged `NXDOMAIN` (`;; Warning: Client COOKIE mismatch`, 4 ms), instead of the real record. Internal names stop resolving whenever v2ray-rs is connected; the start-order workaround only works while dnsmasq still holds answers cached before xray started.

The cause is `autoOutboundsInterface: "auto"` on the generated tun inbound. In Xray-core v26.9.9 on Linux it registers a dialer controller that binds every non-loopback outbound socket with `SO_BINDTODEVICE` to the lowest-metric default-route interface that is not the TUN (`proxy/tun/handler.go` `Start`, `proxy/tun/tun_linux.go` `findOutboundInterface` / `setinterface`). A socket bound that way can only use routes on that interface. Loop prevention does not need it: every dialing xray outbound already carries fwmark 255, and the route helper's pref-9000 rule sends marked traffic to `main`.

## What Changes

- The generated xray tun inbound no longer sets `autoOutboundsInterface`. xray's own traffic keeps bypassing the tunnel through the fwmark and the pref-9000 policy rule, and now follows the host's full `main` routing table, including VPN and other more-specific routes.
- xray loop prevention is specified by the fwmark contract instead of the interface binding.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `tun-mode`: "Outbound loop prevention" — the xray scenario requires the fwmark on every dialing outbound and forbids interface binding.
- `config-generator`: "Generate xray TUN inbound" — the inbound no longer lists `autoOutboundsInterface`.

## Impact

- `crates/core/src/config/v2ray.rs` — `build_xray_tun_inbound` drops the key; tests at `v2ray.rs:1242` and `xray.rs:571` flip to asserting its absence.
- No route helper, persistence, or UI change. Related: `route-xray-exclusions-outside-tun` (excluded routes skip xray entirely), which is complementary — without this change, excluded traffic that still reaches xray keeps egressing the wrong interface.
