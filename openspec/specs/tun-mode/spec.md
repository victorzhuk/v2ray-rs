# Spec: TUN Mode

## Purpose

Defines how the application supports TUN (transparent proxy) mode: which backends support it, how elevated capabilities are acquired, how outbound traffic loops are prevented, and how the privileged route helper manages xray TUN interfaces.
## Requirements
### Requirement: TUN mode availability per backend
The system SHALL offer TUN mode only when the selected backend is sing-box or xray. For the v2ray backend, TUN SHALL be unavailable because v2ray-core has no native TUN inbound. For xray, TUN SHALL additionally require Xray-core v26.1.13 or newer (the first release with the `tun` inbound); starting a TUN connection with an older xray SHALL fail before spawn with an error naming the installed and required versions. For xray versions in the range 26.1.13 through 26.6.22 — affected by the upstream TUN crash on quickly-closed connections (Xray-core #6364, fixed in 26.6.27) — the TUN start SHALL proceed but emit an advisory into the process log stream naming the installed version, the crash behavior, and the fixed version.

#### Scenario: v2ray backend disables TUN
- **WHEN** the selected backend is v2ray and the user opens the TUN settings page
- **THEN** the enable toggle SHALL be insensitive with an explanatory note, and no tun inbound SHALL be generated even if a stale `enabled` flag is persisted

#### Scenario: sing-box and xray expose TUN
- **WHEN** the selected backend is sing-box or xray
- **THEN** the TUN enable toggle SHALL be available, subject to the capability gate

#### Scenario: Old xray blocks TUN start with a clear error
- **WHEN** TUN is enabled and the detected xray version is older than v26.1.13
- **THEN** the connection preflight SHALL fail with an error stating the installed version and the required minimum, without spawning the backend

#### Scenario: Panic-affected xray warns but starts
- **WHEN** TUN is enabled and the detected xray version is at least 26.1.13 but older than 26.6.27
- **THEN** the connection SHALL start normally and a warning log line SHALL appear in the process logs naming the installed version, the quickly-closed-connection crash, and 26.6.27 as the fixed version

### Requirement: TUN requires elevated capabilities granted once
The system SHALL require the backend binary to hold `CAP_NET_ADMIN` before a TUN connection starts, and SHALL detect this by reading the binary's file capabilities. When the capability is missing, the system SHALL offer a one-time grant that runs `setcap` via `pkexec` and SHALL NOT silently start without privileges. The same one-time grant SHALL also ensure that the `v2ray-rs-run` bypass wrapper, when present, is owned by root and carries the setuid bit, so per-process bypass works without a separate elevation.

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
- **WHEN** the backend binary, the route helper, or the `v2ray-rs-run` wrapper resides on a filesystem that does not honor file capabilities or setuid (e.g. mounted `nosuid`)
- **THEN** the grant SHALL fail fast before elevation, naming the affected path and pointing at the manual `setcap` command, instead of reporting success while the privileges silently did not take

### Requirement: Outbound loop prevention
The system SHALL configure each backend so the backend's own outbound traffic bypasses the TUN interface and does not loop.

#### Scenario: sing-box loop prevention
- **WHEN** a sing-box TUN config is generated
- **THEN** the route section SHALL set `auto_detect_interface: true`

#### Scenario: xray loop prevention
- **WHEN** an xray TUN config is generated
- **THEN** the tun inbound settings SHALL set `autoOutboundsInterface: "auto"`

### Requirement: Privileged route helper for xray
The system SHALL include a minimal privileged helper binary that programs and removes the xray TUN routing state, because xray does not configure system routes on Linux. The helper SHALL be idempotent. `xray-up` SHALL ensure the link is up, assign the address(es) ignoring an already-present address, install a default route bound to the TUN device in a dedicated routing table (2023), and install policy rules: fwmark-255 traffic looks up `main` (pref 9000), unmarked traffic looks up `main` with the default route suppressed (`suppress_prefixlength 0`, pref 9001), and everything else looks up the TUN table (pref 9002); with `--bypass-uid`, a uid-range rule to `main` at pref 8998. IPv6 equivalents SHALL be installed when an IPv6 address is supplied.

#### Scenario: Bring xray TUN routes up
- **WHEN** xray has created its TUN device and the helper `xray-up` is invoked with the interface name and address CIDR(s)
- **THEN** the helper SHALL bring the link up, assign the address(es), install the table-2023 default route bound to the device, and install the pref 9000/9001/9002 policy rules (plus the pref 8998 uid-range rule when `--bypass-uid` is given), each step idempotent

#### Scenario: Tear xray TUN routes down
- **WHEN** the helper `xray-down` is invoked for an interface
- **THEN** the helper SHALL remove the policy rules it owns (matching its reserved preferences), delete the device only if it is a TUN device — removing its addresses and device-scoped routes — and SHALL succeed as a no-op when both are already absent

#### Scenario: Recover leftovers after an unclean kill
- **WHEN** a previous TUN connection ended via SIGKILL and the helper `recover` is invoked for the relevant backend
- **THEN** the helper SHALL remove any leftover TUN device and its policy rules, flush its dedicated routing table (2023 for xray), and for sing-box additionally flush the routing rules and table its `auto_route` uses, leaving system networking clean

