## Why

A connection stays "Connected" for as long as the backend process lives, whether or not traffic gets through. An xray TUN session that started 2026-09-14 07:44 UTC ran for about 21 hours; for its last 37 minutes every dial to the node failed with `x509: certificate has expired`, and nothing in the app noticed until the user reconnected by hand. On 2026-09-15 xray logged 1581 `app/dns: failed to retrieve response … context deadline exceeded` errors for DNS sent through the proxy, and the user reconnected five times in under three minutes trying DNS settings. The backend log already contained the evidence; the status bar said Connected throughout.

## What Changes

- While a connection is `Running`, the app periodically requests the configured Real Delay test URL through the session's own local HTTP inbound. Three consecutive failures mark the session unhealthy; the next success marks it healthy again.
- An unhealthy session shows "Proxy not responding" in the status bar and notifies the user once per unhealthy streak (toast, plus a desktop notification when notifications are enabled).
- For xray, a burst of `app/dns: failed to retrieve response` lines marks DNS through the proxy as failing and the status bar says so; other backends are not classified.
- New opt-in setting: when a session started by the configured strategy becomes unhealthy, reconnect with that node excluded. Off by default; never applies to a direct connection to a chosen node.
- Health checks can be turned off in Preferences; they are on by default.

## Capabilities

### New Capabilities

- `connection-health`: active health probing of a running connection, the unhealthy/DNS-failing signals, and user notification.

### Modified Capabilities

- `ui-statusbar-logs`: new requirement for health text in the status bar.
- `connection-auto-resolve`: new requirement for opt-in failover of an unhealthy strategy-planned session.

## Impact

- `crates/subscription/src/health.rs` (new) — single probe through an HTTP proxy with the existing `reqwest` client.
- `crates/ui/src/connection.rs` — per-candidate health monitor task started after `Running`, halted with the forwarders; xray DNS-failure counter on the log stream.
- `crates/ui/src/app.rs` — health state, status text, toast, failover trigger with node exclusion.
- `crates/core/src/models/settings.rs` — `HealthCheckSettings { enabled, failover }` with serde defaults; Preferences toggles on the Network page.
- `crates/tray` — a notification entry point for health messages.
- No new dependency.
