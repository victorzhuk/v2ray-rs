## Why

On a host booted with `ipv6.disable=1`, every sing-box TUN connection dies at startup with `FATAL start service: post-start inbound/tun[tun-in]: starting TUN interface: set rules: add rule 0/9: address family not supported by protocol`, so sing-box TUN is unusable there while xray TUN works. With `strict_route` on and no IPv6 tunnel address, sing-box (sing-tun v0.9.0-beta.4, `tun_linux.go` `rules()`) installs an IPv6 `unreachable` policy rule as its first rule; the kernel has no IPv6 address family, the rule add fails, and sing-box exits. The xray path already skips its IPv6 strict rules when the host has no IPv6; the sing-box path does not.

## What Changes

- When the host kernel has no IPv6 support (`/proc/sys/net/ipv6` absent), a sing-box TUN connection is generated with `strict_route: false`. On such a host that setting only adds the IPv6 unreachable rule, and there is no IPv6 traffic to leak, so nothing is lost.
- The adjustment is announced once per connection in the process log stream, naming the reason.
- A sing-box or xray TUN connection with an IPv6 tunnel address configured on a host without IPv6 fails before spawning the backend, with an error that names the kernel IPv6 state and the setting to clear.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `tun-mode`: new requirement covering TUN startup on hosts whose kernel has IPv6 disabled.

## Impact

- `crates/process/src/tun.rs` — host IPv6 probe shared by the connection layer (netctl keeps its own copy; it stays dependency-light).
- `crates/ui/src/connection.rs` — effective settings for sing-box drop `strict_route` on IPv6-less hosts; notice log line.
- `crates/ui/src/app.rs` — preflight rejection of an IPv6 tunnel address on an IPv6-less host.
- No config schema, persistence, or generator change: the persisted `strict_route` setting is left as the user set it.
