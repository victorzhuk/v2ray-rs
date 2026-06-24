## MODIFIED Requirements

### Requirement: TUN requires elevated capabilities granted once
The system SHALL require the backend binary to hold `CAP_NET_ADMIN` before a TUN
connection starts, and SHALL detect this by reading the binary's file
capabilities. When the capability is missing, the system SHALL offer a one-time
grant that runs `setcap` via `pkexec` and SHALL NOT silently start without
privileges. The same one-time grant SHALL also ensure that the `v2ray-rs-run`
bypass wrapper, when present, is owned by root and carries the setuid bit, so
per-process bypass works without a separate elevation.

#### Scenario: Missing capabilities block TUN start
- **WHEN** TUN is enabled but the backend binary lacks `CAP_NET_ADMIN`
- **THEN** the system SHALL NOT start the backend in TUN mode and SHALL surface the "Grant TUN privileges" action

#### Scenario: One-time grant via pkexec
- **WHEN** the user invokes "Grant TUN privileges"
- **THEN** the system SHALL run a single `pkexec` elevation that applies `cap_net_admin,cap_net_bind_service,cap_net_raw+ep` to the backend binary and `cap_net_admin+ep` to the route helper, sets root ownership and the setuid bit on the `v2ray-rs-run` wrapper when it is present, then re-detect capabilities

#### Scenario: Capabilities lost after upgrade
- **WHEN** the backend binary is replaced (e.g. a package upgrade) and loses its capabilities
- **THEN** the system SHALL detect the missing capability on the next TUN start attempt and re-offer the grant

#### Scenario: File capabilities unsupported
- **WHEN** the backend binary resides on a filesystem that does not honor file capabilities (e.g. mounted `nosuid`)
- **THEN** the system SHALL report a clear error pointing at the manual `setcap` command instead of failing opaquely
