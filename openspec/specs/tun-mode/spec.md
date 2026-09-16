# Spec: TUN Mode

## Purpose

Defines how the application supports TUN (transparent proxy) mode: which backends support it, how elevated capabilities are acquired, how outbound traffic loops are prevented, and how the privileged route helper manages xray TUN interfaces.

## Requirements

### Requirement: TUN mode availability per backend
The system SHALL offer TUN mode only when the selected backend is sing-box or xray. For the v2ray backend, TUN SHALL be unavailable because v2ray-core has no native TUN inbound. When the user connects with the v2ray backend while TUN is enabled in settings, the connection SHALL proceed without TUN and the system SHALL warn the user, by a toast and by one line in the process log stream, that TUN is not supported by v2ray and only applications using the SOCKS or HTTP proxy at the configured listen address and ports are proxied. For xray, TUN SHALL additionally require Xray-core v26.1.13 or newer (the first release with the `tun` inbound); starting a TUN connection with an older xray SHALL fail before spawn with an error naming the installed and required versions. For xray versions in the range 26.1.13 through 26.6.22 — affected by the upstream TUN crash on quickly-closed connections (Xray-core #6364, fixed in 26.6.27) — the TUN start SHALL proceed but emit an advisory into the process log stream naming the installed version, the crash behavior, and the fixed version. When the installed xray version cannot be read or parsed, the start SHALL proceed and the process log SHALL contain a warning that the version could not be read and the minimum-version check was skipped.

#### Scenario: v2ray backend disables TUN
- **WHEN** the selected backend is v2ray and the user opens the TUN settings page
- **THEN** the enable toggle SHALL be insensitive with an explanatory note, and no tun inbound SHALL be generated even if a stale `enabled` flag is persisted

#### Scenario: Connecting with v2ray while TUN is enabled warns
- **WHEN** TUN is enabled in settings, the backend is v2ray, listen address is `127.0.0.1`, SOCKS port 2080, HTTP port 2081, and the user connects
- **THEN** the connection SHALL start without TUN, a warning toast SHALL state that TUN is not supported by v2ray and only apps using SOCKS `127.0.0.1:2080` or HTTP `127.0.0.1:2081` are proxied, and the process log SHALL contain one line with the same warning

#### Scenario: No warning without the TUN flag
- **WHEN** TUN is disabled in settings and the user connects with v2ray, or TUN is enabled and the backend is sing-box or xray
- **THEN** no v2ray TUN warning SHALL be shown or logged

#### Scenario: sing-box and xray expose TUN
- **WHEN** the selected backend is sing-box or xray
- **THEN** the TUN enable toggle SHALL be available, subject to the capability gate

#### Scenario: Old xray blocks TUN start with a clear error
- **WHEN** TUN is enabled and the detected xray version is older than v26.1.13
- **THEN** the connection preflight SHALL fail with an error stating the installed version and the required minimum, without spawning the backend

#### Scenario: Panic-affected xray warns but starts
- **WHEN** TUN is enabled and the detected xray version is at least 26.1.13 but older than 26.6.27
- **THEN** the connection SHALL start normally and a warning log line SHALL appear in the process logs naming the installed version, the quickly-closed-connection crash, and 26.6.27 as the fixed version

#### Scenario: Unreadable xray version warns
- **WHEN** TUN is enabled with xray and its `version` output cannot be read or parsed
- **THEN** the start SHALL continue and the process log SHALL contain a warning that the version could not be read and the minimum-version check was skipped

### Requirement: TUN requires elevated capabilities granted once
The system SHALL require the backend binary to hold `CAP_NET_ADMIN` before a TUN connection starts, and SHALL detect this by reading the binary's file capabilities. For xray, the system SHALL additionally require, before spawning the backend, that the route helper resolves to an existing file this process can execute and that it holds `CAP_NET_ADMIN`. When the application runs with effective user ID 0, capability checks SHALL be skipped. Reading file capabilities SHALL be bounded by a timeout; a timeout or a missing capability-reading tool SHALL fail the start with an error naming the cause. When the backend binary resides on a filesystem that does not honor file capabilities, the start SHALL fail with an error naming the path and the manual `setcap` command. A failure of any of these checks SHALL end the connection attempt without trying further candidates.

#### Scenario: Missing capabilities block TUN start
- **WHEN** TUN is enabled but the backend binary lacks `CAP_NET_ADMIN`
- **THEN** the system SHALL NOT start the backend in TUN mode, SHALL NOT try further candidates, and SHALL surface the "Grant TUN privileges" action with the error

#### Scenario: Route helper lacks its capability
- **WHEN** xray TUN is enabled, the backend holds `CAP_NET_ADMIN`, and the route helper does not
- **THEN** the system SHALL NOT spawn the backend and SHALL surface the "Grant TUN privileges" action

#### Scenario: Route helper not executable yet
- **WHEN** xray TUN is enabled and the relocated route helper exists but this process cannot execute it
- **THEN** the system SHALL NOT spawn the backend and SHALL tell the user to log out and back in to finish enabling TUN

#### Scenario: Route helper missing
- **WHEN** xray TUN is enabled and no route helper can be found
- **THEN** the system SHALL NOT spawn the backend and SHALL report that the route helper was not found

#### Scenario: Running as root
- **WHEN** the application runs with effective user ID 0 and TUN is enabled
- **THEN** the system SHALL NOT read file capabilities and SHALL proceed to start

#### Scenario: Capability probe hangs or is unavailable
- **WHEN** reading file capabilities does not finish within its timeout, or the capability-reading tool is not installed
- **THEN** the start SHALL fail without spawning, with an error stating the timeout or naming the missing tool and its package

#### Scenario: Backend on a nosuid mount
- **WHEN** TUN is enabled and the backend binary resides on a filesystem mounted `nosuid`
- **THEN** the start SHALL fail without spawning, with an error naming the path and the manual `setcap` command, and SHALL NOT offer the grant action

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
The system SHALL configure each backend so the backend's own outbound traffic bypasses the TUN interface and does not loop. For xray, loop prevention SHALL rely on the fwmark carried by every dialing outbound and the route helper's policy rule sending marked traffic to the `main` table; the generated config SHALL NOT bind outbound sockets to a fixed or auto-detected interface, so xray's egress follows every route in `main`, including more-specific routes through other interfaces such as a VPN.

#### Scenario: sing-box loop prevention
- **WHEN** a sing-box TUN config is generated
- **THEN** the route section SHALL set `auto_detect_interface: true`

#### Scenario: xray loop prevention
- **WHEN** an xray TUN config is generated
- **THEN** every outbound other than `blackhole` and `dns` SHALL set `streamSettings.sockopt.mark` to 255, and the tun inbound settings SHALL NOT contain `autoOutboundsInterface`

#### Scenario: xray direct egress follows VPN routes
- **WHEN** xray TUN is connected, a VPN interface holds a more-specific route such as `10.0.0.0/8`, and traffic to an address in that range is routed by xray to `direct`
- **THEN** that traffic SHALL leave through the VPN interface, not the default-route interface

### Requirement: Privileged route helper for xray
The system SHALL include a minimal privileged helper binary that programs and removes the xray TUN routing state, because xray does not configure system routes on Linux. The helper SHALL be idempotent. `xray-up` SHALL ensure the link is up, assign the address(es) ignoring an already-present address, install a default route bound to the TUN device in a dedicated routing table (2023), and install policy rules: fwmark-255 traffic looks up `main` (pref 9000), unmarked traffic looks up `main` with the default route suppressed (`suppress_prefixlength 0`, pref 9001), and everything else looks up the TUN table (pref 9002); with `--bypass-uid`, a uid-range rule to `main` at pref 8998; with `--capture-dns`, unmarked udp and tcp traffic to port 53 looks up the TUN table (pref 8999), so a resolver on the local subnet is reached through the tunnel rather than the LAN route pref 9001 preserves. IPv6 equivalents SHALL be installed when an IPv6 address is supplied. With `--strict`, `xray-up` SHALL additionally install an `unreachable` default route with the lowest priority in table 2023 for IPv4 and IPv6, and SHALL install the IPv6 policy rules even when no IPv6 address is supplied — unless the host has IPv6 disabled, in which case the IPv6 fallback route and rules are skipped — so that traffic destined for the tunnel is refused rather than routed through the real default route whenever the TUN device is absent. Re-running `xray-up` against a recreated device of the same name SHALL succeed and leave exactly one copy of each route and rule.

#### Scenario: Bring xray TUN routes up
- **WHEN** xray has created its TUN device and the helper `xray-up` is invoked with the interface name and address CIDR(s)
- **THEN** the helper SHALL bring the link up, assign the address(es), install the table-2023 default route bound to the device, and install the pref 9000/9001/9002 policy rules (plus the pref 8998 uid-range rule when `--bypass-uid` is given and the pref 8999 port-53 rules when `--capture-dns` is given), each step idempotent

#### Scenario: DNS capture excludes the backend's own queries
- **WHEN** the pref 8999 rules are installed
- **THEN** they SHALL match only unmarked traffic, so the backend's own resolver queries keep egressing the real interface through the pref 9000 rule

#### Scenario: Strict fallback routes
- **WHEN** `xray-up` is invoked with `--strict`
- **THEN** table 2023 SHALL contain an `unreachable` default route for IPv4 and for IPv6 at a lower priority than the device route, and the IPv6 pref 9000/9001/9002 rules SHALL be present whether or not an IPv6 address was supplied, on any host with IPv6 enabled

#### Scenario: Device gone under strict route
- **WHEN** the strict routing state is installed and the TUN device disappears
- **THEN** unmarked traffic to a destination outside the on-link routes SHALL be refused with host-unreachable rather than sent through the real default route, while fwmark-255 and bypass-uid traffic SHALL keep using `main`

#### Scenario: Re-running up across a recreated device
- **WHEN** `xray-up` has run, the TUN device is deleted and recreated with the same name, and `xray-up` runs again
- **THEN** the helper SHALL succeed and the resulting routes and rules SHALL match a single fresh `xray-up`

#### Scenario: Tear xray TUN routes down
- **WHEN** the helper `xray-down` is invoked for an interface
- **THEN** the helper SHALL remove the policy rules it owns (matching its reserved preferences) for both address families, remove the table-2023 fallback routes, delete the device only if it is a TUN device — removing its addresses and device-scoped routes — and SHALL succeed as a no-op when all are already absent

#### Scenario: Recover leftovers after an unclean kill
- **WHEN** a previous TUN connection ended via SIGKILL and the helper `recover` is invoked for the relevant backend
- **THEN** the helper SHALL remove any leftover TUN device and its policy rules, flush its dedicated routing table (2023 for xray) including the fallback routes, and for sing-box additionally flush the routing rules and table its `auto_route` uses, leaving system networking clean

### Requirement: Proxy hostname resolution is bootstrapped
Before a TUN session's routes exist, the system SHALL resolve every hostname-addressed proxy node through the operating-system resolver and carry every answer, of both families, into the generated config as static host overrides, so the backend never has to resolve its own server through the tunnel it is building; each generator keeps the addresses its backend can use. Kernel-side DNS capture SHALL be armed only when the generated config actually carries an override the backend can answer with for every hostname-addressed node, because capturing port 53 while the backend still needs a name resolved sends that lookup into the tunnel. The TUN runtime SHALL be built from the same effective settings the config was generated from.

#### Scenario: Hostname node is pinned before the tunnel exists
- **WHEN** a connection starts with TUN enabled and the selected node is addressed by a hostname
- **THEN** the system SHALL resolve it through the operating-system resolver before the route helper runs, and the generated config SHALL contain a host override for that hostname

#### Scenario: Unusable answers do not count as resolved
- **WHEN** a hostname resolves only to addresses of a family xray's query strategy will not use
- **THEN** the node SHALL count as unpinned and DNS capture SHALL stay off

#### Scenario: Capture stays off when the config carries no override
- **WHEN** any hostname-addressed node has no host override in the generated config
- **THEN** the route helper SHALL be invoked without DNS capture, and the session SHALL still start

#### Scenario: Route helper and config agree
- **WHEN** a runtime profile overrides TUN settings for the connection
- **THEN** the route helper SHALL be configured from the same effective settings used to generate the config

### Requirement: Traffic stays blocked while an xray TUN session reconnects
When the backend is xray, TUN is enabled and `strict_route` is on, the system SHALL keep the session's strict routing state installed from the moment the backend exits unexpectedly until the session is running again or the system stops trying — across in-place respawns, failover to another candidate, and automatic reconnect attempts. The system SHALL release it, restoring the host's normal routing, on Disconnect, on Quit, and when automatic reconnects are exhausted. With `strict_route` off, the system SHALL install no fallback routes and no IPv6 rules without an IPv6 address, preserving the previous behavior.

#### Scenario: Crash respawn does not leak
- **WHEN** an xray TUN backend with `strict_route` on crashes and is being respawned
- **THEN** no unmarked traffic outside the on-link routes SHALL leave through the real default route until the respawned backend's routes are up

#### Scenario: Blocked across automatic reconnects
- **WHEN** every candidate has failed and an automatic reconnect is scheduled
- **THEN** the strict routing state SHALL remain installed until that reconnect succeeds or the automatic reconnects are exhausted

#### Scenario: Released on final give-up
- **WHEN** automatic reconnects are exhausted and the connection is left in `Error`
- **THEN** the system SHALL remove the session's routing state and the TUN recovery marker, restoring normal connectivity

#### Scenario: Released on Disconnect without a live backend
- **WHEN** the user invokes Disconnect while the connection is in `Error` with the strict routing state still installed
- **THEN** the system SHALL remove the routing state and the marker and report `Stopped`

#### Scenario: IPv6 without a tunnel address
- **WHEN** xray TUN starts with `strict_route` on and no IPv6 tunnel address
- **THEN** IPv6 traffic to destinations outside the on-link routes SHALL be refused rather than sent through the real default route

#### Scenario: Host without IPv6
- **WHEN** xray TUN starts with `strict_route` on, no IPv6 tunnel address, and IPv6 disabled on the host
- **THEN** the route helper SHALL skip the IPv6 fallback route and IPv6 policy rules and still install the IPv4 strict state; on a host with IPv6 available, any failure to install the IPv6 strict state SHALL fail the start

#### Scenario: Strict route off keeps previous behavior
- **WHEN** xray TUN runs with `strict_route` off
- **THEN** the route helper SHALL be invoked without `--strict`, and IPv6 SHALL be routed into the tunnel only when an IPv6 tunnel address is set

### Requirement: TUN starts on hosts with kernel IPv6 disabled
The system SHALL start TUN connections on a host whose kernel has no IPv6 support (booted with `ipv6.disable=1`). For sing-box, the connection's generated config SHALL set `strict_route: false` on such a host regardless of the persisted setting, because sing-box's strict route adds an IPv6 policy rule the kernel rejects, and SHALL write one notice line to the process log stream stating that kernel IPv6 is disabled and strict route was turned off for the session. The persisted `strict_route` setting SHALL NOT change. When an IPv6 tunnel address is configured on such a host, a TUN connection for either backend SHALL fail before any backend is spawned, with an error stating that the kernel has IPv6 disabled and that the IPv6 tunnel address must be cleared.

#### Scenario: sing-box strict route on an IPv6-less host
- **WHEN** a sing-box TUN connection starts with `strict_route` on, no IPv6 tunnel address, and kernel IPv6 disabled
- **THEN** the generated tun inbound SHALL contain `"strict_route": false`, the process log SHALL contain one notice naming disabled kernel IPv6, and the persisted settings SHALL still have `strict_route` on

#### Scenario: sing-box strict route on a host with IPv6
- **WHEN** a sing-box TUN connection starts with `strict_route` on and kernel IPv6 available (including with `net.ipv6.conf.all.disable_ipv6=1`)
- **THEN** the generated tun inbound SHALL contain `"strict_route": true` and no notice SHALL be logged

#### Scenario: IPv6 tunnel address on an IPv6-less host
- **WHEN** TUN is enabled with an IPv6 tunnel address, the backend is sing-box or xray, and kernel IPv6 is disabled
- **THEN** Connect SHALL fail without spawning a backend or trying any candidate, and the user SHALL see an error naming disabled kernel IPv6 and the IPv6 tunnel address setting
