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
The system SHALL include a minimal privileged helper binary that programs and removes the xray TUN routing state, because xray does not configure system routes on Linux. The helper SHALL be idempotent. `xray-up` SHALL ensure the link is up, assign the address(es) ignoring an already-present address, install a default route bound to the TUN device in a dedicated routing table (2023), and install policy rules: fwmark-255 traffic looks up `main` (pref 9000), unmarked traffic looks up `main` with the default route suppressed (`suppress_prefixlength 0`, pref 9001), and everything else looks up the TUN table (pref 9002); with `--bypass-uid`, a uid-range rule to `main` at pref 8998; with `--capture-dns`, unmarked udp and tcp traffic to port 53 looks up the TUN table (pref 8999), so a resolver on the local subnet is reached through the tunnel rather than the LAN route pref 9001 preserves. With one or more `--exclude <CIDR>` arguments, `xray-up` SHALL install, for each CIDR, a rule sending traffic destined to that prefix to `main` at pref 8997, ahead of DNS capture and the tunnel table, so excluded destinations — including resolvers on port 53 — never enter the TUN device; the pref 8997 rule set SHALL be replaced on every `xray-up`, so exclusions removed since the previous run do not linger. Each `--exclude` value SHALL be validated as an IPv4 or IPv6 CIDR before any netlink call, host bits SHALL be cleared, the number of values SHALL be bounded, and IPv6 exclusions SHALL be skipped when the host has IPv6 disabled. IPv6 equivalents SHALL be installed when an IPv6 address is supplied. With `--strict`, `xray-up` SHALL additionally install an `unreachable` default route with the lowest priority in table 2023 for IPv4 and IPv6, and SHALL install the IPv6 policy rules even when no IPv6 address is supplied — unless the host has IPv6 disabled, in which case the IPv6 fallback route and rules are skipped — so that traffic destined for the tunnel is refused rather than routed through the real default route whenever the TUN device is absent. Re-running `xray-up` against a recreated device of the same name SHALL succeed and leave exactly one copy of each route and rule.

#### Scenario: Bring xray TUN routes up
- **WHEN** xray has created its TUN device and the helper `xray-up` is invoked with the interface name and address CIDR(s)
- **THEN** the helper SHALL bring the link up, assign the address(es), install the table-2023 default route bound to the device, and install the pref 9000/9001/9002 policy rules (plus the pref 8998 uid-range rule when `--bypass-uid` is given, the pref 8999 port-53 rules when `--capture-dns` is given, and one pref 8997 rule per `--exclude` CIDR), each step idempotent

#### Scenario: DNS capture excludes the backend's own queries
- **WHEN** the pref 8999 rules are installed
- **THEN** they SHALL match only unmarked traffic, so the backend's own resolver queries keep egressing the real interface through the pref 9000 rule

#### Scenario: Excluded destination bypasses the tunnel
- **WHEN** `xray-up` runs with `--exclude 91.230.107.224/32` and the host's `main` table has a default route via the physical interface
- **THEN** `ip route get 91.230.107.224` for an unmarked packet SHALL resolve through `main` to the physical interface, not the TUN device

#### Scenario: Excluded resolver skips DNS capture
- **WHEN** `xray-up` runs with `--capture-dns` and `--exclude 10.15.12.100/32`
- **THEN** unmarked udp traffic to `10.15.12.100:53` SHALL match the pref 8997 rule before the pref 8999 capture rule and SHALL NOT enter the TUN device

#### Scenario: Exclusions replaced on re-run
- **WHEN** `xray-up` ran with `--exclude 10.0.0.0/8 --exclude 192.0.2.0/24` and runs again with only `--exclude 10.0.0.0/8`
- **THEN** exactly one pref 8997 rule SHALL exist, for `10.0.0.0/8`

#### Scenario: Invalid exclusion refused
- **WHEN** `xray-up` is invoked with `--exclude 10.0.0.0/33` or a non-CIDR value
- **THEN** the helper SHALL exit with an error naming the value and SHALL make no netlink change

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
- **THEN** the helper SHALL remove the policy rules it owns (matching its reserved preferences, including pref 8997) for both address families, remove the table-2023 fallback routes, delete the device only if it is a TUN device — removing its addresses and device-scoped routes — and SHALL succeed as a no-op when all are already absent

#### Scenario: Recover leftovers after an unclean kill
- **WHEN** a previous TUN connection ended via SIGKILL and the helper `recover` is invoked for the relevant backend
- **THEN** the helper SHALL remove any leftover TUN device and its policy rules, flush its dedicated routing table (2023 for xray) including the fallback routes, and for sing-box additionally flush the routing rules and table its `auto_route` uses, leaving system networking clean

### Requirement: Proxy hostname resolution is bootstrapped
For TUN connections only, the system SHALL resolve every hostname-addressed proxy node through the operating-system resolver and carry the answers, of both families, into the generated config as static host overrides, so the backend never has to resolve its own server through the tunnel it is building; each generator keeps the addresses its backend can use. Connections without TUN SHALL NOT pin node hostnames. The hostnames of all candidates of a connection attempt, including nodes their routing rules send traffic through, SHALL be resolved once, before the first candidate is started, and never while routing state installed by an earlier candidate of the same attempt is present. Before a candidate starts, the system SHALL test each of its pinned addresses with a TCP connection to the node's port, bounded by a short timeout, and SHALL keep only the addresses that accepted; when none accepted, or when a previously failed candidate's routing state is still installed, it SHALL keep all of them. When a hostname cannot be resolved, the system SHALL use the addresses pinned for that hostname by the most recent session of the running application that reached `Running`, if any. Kernel-side DNS capture SHALL be armed only when the generated config actually carries an override the backend can answer with for every hostname-addressed node, because capturing port 53 while the backend still needs a name resolved sends that lookup into the tunnel. When capture would otherwise be armed but a node is unpinned, the system SHALL notify the user once per connection attempt, write a notice naming the hostname to the process log stream, and record in the session record that DNS capture is off. The TUN runtime SHALL be built from the same effective settings the config was generated from.

#### Scenario: Hostname node is pinned before the tunnel exists
- **WHEN** a connection starts with TUN enabled and the selected node is addressed by a hostname
- **THEN** the system SHALL resolve it through the operating-system resolver before the route helper runs, and the generated config SHALL contain a host override for that hostname

#### Scenario: No pinning without TUN
- **WHEN** a connection starts with TUN disabled and the selected node is addressed by a hostname
- **THEN** the system SHALL NOT resolve it in advance and the generated config SHALL contain no host override for it that the user did not configure

#### Scenario: Failover does not resolve through a parked tunnel
- **WHEN** an xray TUN candidate fails, its routing state stays installed, and the next candidate is addressed by a hostname
- **THEN** that hostname SHALL already have been resolved before the first candidate started, and no lookup SHALL be made while the failed candidate's routing state is installed

#### Scenario: Unreachable pinned addresses are dropped
- **WHEN** a hostname resolves to three addresses and only one accepts a TCP connection on the node's port
- **THEN** the generated config SHALL pin only that address

#### Scenario: No address answers
- **WHEN** none of a hostname's resolved addresses accepts a TCP connection on the node's port
- **THEN** the generated config SHALL pin all resolved addresses

#### Scenario: Last good addresses cover a failed lookup
- **WHEN** a session pinned `proxy.example.com` and reached `Running`, and a later automatic reconnect cannot resolve that hostname
- **THEN** the reconnect SHALL pin the addresses the earlier session used

#### Scenario: Unusable answers do not count as resolved
- **WHEN** a hostname resolves only to addresses of a family xray's query strategy will not use
- **THEN** the node SHALL count as unpinned and DNS capture SHALL stay off

#### Scenario: Capture stays off when the config carries no override
- **WHEN** any hostname-addressed node has no host override in the generated config
- **THEN** the route helper SHALL be invoked without DNS capture, and the session SHALL still start

#### Scenario: Disabled capture is visible
- **WHEN** xray TUN with DNS hijack enabled starts a candidate whose hostname could not be pinned
- **THEN** the user SHALL see a notification that DNS capture is off for the session, the process log SHALL contain a notice naming the hostname, and the session record SHALL state that DNS capture is off

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

### Requirement: xray TUN connections pass excluded routes to the route helper
For an xray TUN connection, the system SHALL pass every entry of the effective `tun.exclude_routes` to the route helper as an exclusion, built from the same effective settings the config was generated from. The generated xray routing rule sending those CIDRs to `direct` SHALL remain. sing-box connections SHALL keep expressing exclusions through `route_exclude_address` and SHALL NOT invoke the helper for them. Each excluded route SHALL be refused by settings validation when its prefix length is 0, and more than 256 excluded routes SHALL be refused, so the TUN preferences reject such an entry before it is saved and config generation fails with an error naming the value instead of the helper refusing it at connect.

#### Scenario: Excluded routes reach the helper
- **WHEN** an xray TUN connection starts with `exclude_routes` `["10.15.12.100/32", "91.230.107.224/32"]`
- **THEN** the route helper SHALL be invoked with `--exclude 10.15.12.100/32 --exclude 91.230.107.224/32`

#### Scenario: No exclusions
- **WHEN** an xray TUN connection starts with an empty `exclude_routes`
- **THEN** the route helper SHALL be invoked without `--exclude`

#### Scenario: Whole-address-space exclusion is refused
- **WHEN** the user enters `0.0.0.0/0` or `::/0` as an excluded route
- **THEN** the TUN preferences SHALL reject the entry, and settings carrying it SHALL fail validation with an error naming the value

#### Scenario: Too many exclusions are refused
- **WHEN** settings carry 257 excluded routes
- **THEN** settings validation SHALL fail
