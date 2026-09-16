## Why

With TUN enabled in settings and the backend switched to v2ray, Connect succeeds (`session backend=v2ray … tun=off`, `V2Ray 5.52.0 started`), but system traffic is no longer proxied: v2ray-core has no TUN inbound, so only apps configured for the local SOCKS/HTTP ports use the proxy. Nothing at connect time says so. The only hint is an insensitive toggle subtitle in Preferences, and the v2ray config logs at `warning`, so the logs page stays silent too. The user sees "v2ray does not work at all".

## What Changes

- Connecting with the v2ray backend while TUN is enabled in settings shows a warning toast. It says TUN is not supported by v2ray and that only apps using the SOCKS or HTTP proxy, at the configured listen address and ports, go through the proxy.
- The same warning is written as one line to the connection's log stream, so it is visible on the logs page and in `backend.log`.
- The session still connects; no setting is changed.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `tun-mode`: "TUN mode availability per backend" gains a connect-time warning for v2ray with a stale TUN flag.

## Impact

- `crates/ui/src/app.rs` — `start_connection` warning toast.
- `crates/ui/src/connection.rs` — warning line into the log stream at session start.
