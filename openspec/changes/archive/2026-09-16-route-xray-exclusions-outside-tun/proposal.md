## Why

On xray TUN, "Excluded routes" does not keep traffic out of the tunnel. The generator only adds a routing rule `{"ip": [...], "outboundTag": "direct"}`; the route helper installs no rule for those destinations, so the kernel still sends them into the TUN device and xray re-emits them through `freedom`. With `10.15.12.100/32` (a corporate resolver) in the exclusion list, `backend.log` holds hundreds of `accepted udp:10.15.12.100:53 [tun-in -> direct]` lines. Every excluded packet pays a userspace round trip through xray, a VPN whose server a user excludes still rides on xray's UDP handling (the same log has thousands of `udp:91.230.107.224:1194 [tun-in -> direct]` lines for a work VPN endpoint routed direct), and DNS capture (pref 8999) pulls an excluded resolver on port 53 into the tunnel before any exclusion can apply. sing-box does not have this problem: its tun inbound carries `route_exclude_address`, which keeps those prefixes out of its `auto_route` rules.

## What Changes

- `v2ray-rs-netctl xray-up` accepts repeated `--exclude <CIDR>` and installs a policy rule per prefix sending it to `main` at pref 8997, ahead of the bypass-uid, DNS capture, and tunnel rules. The set is replaced on every `xray-up`; `xray-down` and `recover` remove it with the other reserved-preference rules.
- Values are validated before any netlink call (CIDR syntax, prefix length 1..=32/128, host bits cleared, bounded count); IPv6 prefixes are skipped on hosts with IPv6 disabled.
- The xray TUN runtime carries the effective `tun.exclude_routes` and passes them to the helper. The generated `ip → direct` routing rule stays.
- The TUN preferences page describes exclusion lists per backend: excluded routes leave the tunnel on both; excluded domains on xray still enter the tunnel and are sent direct by xray.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `tun-mode`: "Privileged route helper for xray" gains the pref 8997 exclusion rules; new requirement that xray TUN connections pass excluded routes to the helper.
- `tun-preferences-ui`: new requirement for per-backend exclusion wording.

## Impact

- `crates/netctl/src/main.rs`, `crates/netctl/src/net.rs`, `crates/netctl/src/validate.rs` — new argument, rule install/replace/teardown, validation; `crates/netctl/tests/privileged.rs` namespace tests.
- `crates/process/src/tun.rs` — `TunRuntime::exclude_routes`, `xray_up_args`.
- `crates/ui/src/connection.rs` — `build_tun_runtime`.
- `crates/ui/src/preferences/tun.rs` — group descriptions.
- Route helper CLI grows an argument; the app and helper ship together, so no compatibility shim.
