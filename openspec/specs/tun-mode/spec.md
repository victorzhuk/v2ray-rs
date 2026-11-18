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
The system SHALL require the backend binary to hold `CAP_NET_ADMIN` before a TUN connection starts, and SHALL detect this by reading the binary's file capabilities. For xray, the system SHALL additionally require, before spawning the backend, that the exact stored route helper path is ready and that it holds `CAP_NET_ADMIN`. Readiness SHALL be decided by one shared classifier over that stored path — never a fresh path resolution — producing exactly one of: ready, missing, enrollment required, re-login required, wrong ownership or mode, a `noexec` mount, or a failed access probe. Failed mount, group, or execution lookups SHALL be reported as a failed probe and SHALL NOT be reported as a need to log out. Only the account being enrolled in `v2ray-rs` while the running session does not yet hold that group SHALL ask the user to log out and back in, and that one genuine case remains, because route-helper access is group-only. The connection preflight and the TUN preferences page SHALL use the same classifier and the same resolved path, so their verdicts cannot disagree. Every non-ready classification SHALL map to a host-level error and SHALL end the connection attempt without trying further candidates. When the application runs with effective user ID 0, capability checks SHALL be skipped. Reading file capabilities SHALL be bounded by a timeout; a timeout or a missing capability-reading tool SHALL fail the start with an error naming the cause. When the backend binary resides on a filesystem that does not honor file capabilities, the start SHALL fail with an error naming the path and the manual `setcap` command. A failure of any of these checks SHALL end the connection attempt without trying further candidates.

#### Scenario: Missing capabilities block TUN start
- **WHEN** TUN is enabled but the backend binary lacks `CAP_NET_ADMIN`
- **THEN** the system SHALL NOT start the backend in TUN mode, SHALL NOT try further candidates, and SHALL surface the "Grant TUN privileges" action with the error

#### Scenario: Route helper lacks its capability
- **WHEN** xray TUN is enabled, the backend holds `CAP_NET_ADMIN`, and the route helper does not
- **THEN** the system SHALL NOT spawn the backend and SHALL surface the "Grant TUN privileges" action

#### Scenario: Route helper not executable yet
- **WHEN** xray TUN is enabled, the stored route helper exists with correct `root:v2ray-rs` `0750` ownership and mode, the invoking account is in `v2ray-rs`, and the running session does not hold that group yet
- **THEN** the system SHALL NOT spawn the backend and SHALL tell the user to log out and back in to finish enabling TUN

#### Scenario: Wrong helper ownership or mode is not a logout
- **WHEN** the stored route helper exists but is not owned by `root:v2ray-rs` with mode `0750`
- **THEN** the start SHALL fail without spawning, with an error naming the required ownership and mode and pointing at the grant action, and SHALL NOT ask the user to log out

#### Scenario: Account not enrolled in the helper group
- **WHEN** the stored route helper has correct ownership and mode and the dedicated group exists, but the invoking account is not a member of it
- **THEN** the start SHALL fail without spawning, with an error naming the required group membership and the grant action

#### Scenario: Helper on a noexec filesystem
- **WHEN** the stored route helper resides on a filesystem mounted `noexec`
- **THEN** the start SHALL fail without spawning, with an error naming the path and the filesystem, and SHALL NOT ask the user to log out or to re-grant

#### Scenario: Helper access cannot be probed
- **WHEN** the mount options covering the helper cannot be read, the helper group cannot be resolved, or the execution check fails with an unexpected error
- **THEN** the start SHALL fail without spawning with an error stating that route-helper access could not be verified and the underlying detail, and SHALL NOT ask the user to log out

#### Scenario: Startup and preferences agree
- **WHEN** the stored route helper is classified by the connection preflight and the TUN preferences page reads the same helper
- **THEN** both SHALL report the same classification and the same resolved path, and neither SHALL re-resolve a different helper

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
- **THEN** the system SHALL run a single `pkexec` elevation that applies `cap_net_admin,cap_net_bind_service,cap_net_raw+ep` to the backend binary and `cap_net_admin+ep` to the route helper, enrolls the invoking account in the `v2ray-rs` group, sets `root:v2ray-rs` ownership before mode `0750` on the route helper and mode `4750` on the `v2ray-rs-run` wrapper when it is present, applies every ownership change before every mode change and every capability change, and then verifies the installed result by identity rather than trusting the exit status

#### Scenario: Grant refuses unsafe mutation targets
- **WHEN** an existing mutation target is a symlink or not a regular file, is not root-owned, or is writable by non-root accounts, or one of its parent directories is not a protected root-owned directory, or its recorded `device:inode` identity changed between the pre-elevation check and the elevated script
- **THEN** the grant SHALL refuse before any mutation, naming the path and the reason and pointing at installing the distribution package or asking an administrator, and SHALL NOT promote a user-owned or user-writable file through ownership changes

#### Scenario: Grant refuses a root caller
- **WHEN** the application runs with real or effective user ID 0 and a grant is invoked
- **THEN** the grant SHALL refuse to enroll uid 0 before planning or elevating, telling the user to run the application as a non-root account

#### Scenario: Group-only access is preserved, ACLs untouched
- **WHEN** the helper or the wrapper carries access ACL entries
- **THEN** the grant SHALL preserve existing entries, SHALL refuse a mode change that would activate dormant named entries, SHALL refuse when the access ACL cannot be verified, and SHALL NOT add or remove any ACL entry

#### Scenario: Grant verifies the installed result
- **WHEN** the elevation exits successfully
- **THEN** the system SHALL verify the installed helper's identity, regular-file status, `root:v2ray-rs` ownership, mode `0750`, capability, and the account's enrollment, the wrapper's `root:v2ray-rs` ownership and mode `4750` when present, and the backend's unchanged identity and capability, and SHALL report the grant as unverified rather than successful when any check fails

#### Scenario: Relocated helper is verified by its new identity
- **WHEN** a relocating grant installs a fresh copy of the route helper
- **THEN** the elevated script SHALL emit exactly one `helper-identity=<device>:<inode>` record, the system SHALL verify the installed copy against that identity rather than the replaced inode, and SHALL fail verification on missing, malformed, additional, or mismatched output

#### Scenario: Genuine enrollment needs one log-out
- **WHEN** a grant enrolls the account in the `v2ray-rs` group
- **THEN** the system SHALL state that one log-out and log-in is required before the running session can execute the helper, and SHALL NOT report the current session as ready

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
For TUN connections only, the system SHALL resolve hostname-addressed proxy nodes through the OS resolver before tunnel routes block unmarked lookups, carrying both address families into static host overrides while connections without TUN never pin node hostnames. When no fail-closed TUN routing state is installed (`sing-box` TUN or non-strict `xray` TUN), hostnames SHALL be resolved lazily per candidate; before strict `xray` TUN installs routes, pre-pinning SHALL be bounded to at most 8 planned candidates (and their `via_node`s). When dead fail-closed routes from `Error` remain installed, reconnect SHALL skip live OS lookups and TCP pin probes in favor of `last_good_pins`, and `Stop` SHALL cancel in-flight lookups and probes immediately. Kernel DNS capture SHALL be armed only when every hostname-addressed node of the candidate has a usable override, notifying and logging once per attempt when capture stays off, with the TUN runtime built from the same effective settings as the config.

#### Scenario: Hostname node is pinned before the tunnel exists
- **WHEN** a connection starts with TUN enabled and the selected node is addressed by a hostname
- **THEN** the system SHALL resolve it through the operating-system resolver before the route helper runs, and the generated config SHALL contain a host override for that hostname

#### Scenario: No pinning without TUN
- **WHEN** a connection starts with TUN disabled and the selected node is addressed by a hostname
- **THEN** the system SHALL NOT resolve it in advance and the generated config SHALL contain no host override for it that the user did not configure

#### Scenario: Lazy per-candidate resolution when no routes stay parked
- **WHEN** a `sing-box` TUN or non-strict `xray` TUN connection starts across 50 hostname-addressed subscription nodes and candidate 1 succeeds
- **THEN** only candidate 1 (and any `via_node` it uses) SHALL be resolved through the OS resolver, without disclosing the remaining 49 hostnames

#### Scenario: Failover does not resolve through a parked tunnel
- **WHEN** a strict `xray` TUN candidate fails, its routing state stays installed, and failover advances within the bounded lookahead (`<= 8` candidates)
- **THEN** the next candidate's hostname SHALL already have been resolved before the first candidate installed routes, and no live OS lookup SHALL run while strict routing state is installed

#### Scenario: Reconnect under parked strict routes uses last good pins immediately
- **WHEN** strict `xray` TUN is parked in `Error` with dead fail-closed routes installed and an automatic reconnect starts
- **THEN** the reconnect SHALL skip live `os_lookup` and `probe_pins` timeouts and immediately use `last_good_pins`

#### Scenario: Stop cancels in-flight hostname lookup or pin probe
- **WHEN** the user invokes Disconnect while `os_lookup` or `probe_pins` is awaiting network I/O
- **THEN** the lookup or probe SHALL abort immediately and the session SHALL transition to `Stopped` without waiting for resolver or TCP probe timeouts

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
For an xray TUN connection, the system SHALL pass every entry of the effective `tun.exclude_routes` to the route helper as an exclusion, built from the same effective settings the config was generated from. The generated xray routing rule sending those CIDRs to `direct` SHALL remain. sing-box connections SHALL keep expressing exclusions through `route_exclude_address` and SHALL NOT invoke the helper for them. Each excluded route SHALL be refused by settings validation when its prefix length is 0, and more than 256 excluded routes SHALL be refused, so the TUN preferences reject such an entry before it is saved and config generation fails with an error naming the value instead of the helper refusing it at connect. This limit applies to saved settings only; the automatic connected-network entries described below SHALL NOT count toward it.

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

### Requirement: sing-box TUN excludes the host's own attached networks
For a sing-box TUN connection, the system SHALL collect, without privileges, the prefixes of the interfaces directly attached to the host and merge them into the generated `tun` inbound's `route_exclude_address`, after validating saved settings, so return traffic for an attached network — a Docker bridge, a LAN — is not captured by `auto_route`. Collection SHALL accept IPv4 and IPv6 records of interfaces that are up, SHALL skip loopback and point-to-point interfaces, the configured TUN interface, and the `/0`, `/32`, and `/128` prefixes, SHALL require a same-family contiguous netmask and fail with the interface named when one is missing or invalid, and SHALL return the prefixes sorted and deduplicated. The system SHALL NOT exclude ranges the host is not attached to, such as blanket private address space. Explicit user entries SHALL keep their order and their content, including redundancy; automatic prefixes SHALL be appended, skipping any that duplicate an explicit entry and any whose address family the TUN does not carry. `settings.tun.exclude_routes` SHALL NOT be modified. The snapshot SHALL be taken once per connection before any candidate starts and reused across candidates and crash respawns, and again on a disconnected config regeneration; it is not a route monitor, so a network created after the snapshot is excluded only on the next connect. A collection failure SHALL end the connection with one error naming the failure before any config write, recovery marker, or backend spawn, and no other node SHALL be attempted; a failed regeneration SHALL keep the previously generated file. sing-box SHALL continue to program its own routes and SHALL NOT invoke the privileged route helper, so a sing-box TUN start still requires no route helper.

#### Scenario: Attached bridge prefix is excluded
- **WHEN** a sing-box TUN connection starts on a host where `docker0` is up with prefix `172.17.0.0/16`
- **THEN** the generated `route_exclude_address` SHALL contain `172.17.0.0/16` alongside any explicit exclusions, and the connection SHALL NOT invoke the route helper

#### Scenario: Loopback, point-to-point, and TUN prefixes are skipped
- **WHEN** the host has a loopback address, a point-to-point interface address, and an address on the configured TUN interface
- **THEN** none of those prefixes SHALL appear in the generated `route_exclude_address`

#### Scenario: Down interfaces are skipped and host routes dropped
- **WHEN** an interface is down, or a record's prefix is `/0`, `/32`, or `/128`
- **THEN** that record SHALL contribute no exclusion entry

#### Scenario: Invalid mask fails the collection
- **WHEN** an eligible interface record has no netmask, or one of the wrong family, or a non-contiguous one
- **THEN** the collection SHALL fail naming the interface and the cause

#### Scenario: Explicit entries keep order and redundancy
- **WHEN** `exclude_routes` holds explicit entries and the snapshot overlaps one of them
- **THEN** every explicit entry SHALL remain in its original order, duplicates included, and only the overlapping automatic entry SHALL be dropped

#### Scenario: Inactive address family is not excluded
- **WHEN** the TUN carries no IPv6 address and the snapshot contains IPv6 prefixes
- **THEN** no IPv6 prefix SHALL be added to `route_exclude_address`

#### Scenario: Saved exclusions are not rewritten
- **WHEN** automatic entries are merged into a generated config
- **THEN** `settings.tun.exclude_routes` SHALL still hold exactly the entries the user configured

#### Scenario: One snapshot serves every candidate
- **WHEN** a sing-box TUN connection fails over to another candidate or respawns after a crash
- **THEN** every candidate's generated config SHALL carry the same snapshot, collected once before the first candidate

#### Scenario: Stop during collection
- **WHEN** the user requests Stop while the connected-network snapshot is still being collected
- **THEN** no candidate SHALL start and the connection SHALL report `Stopped`

#### Scenario: Collection failure ends the attempt
- **WHEN** collecting the connected-network prefixes fails
- **THEN** the connection SHALL report one error beginning with the collection failure, and no config file, recovery marker, or backend process SHALL be created, and no further node SHALL be attempted

#### Scenario: Regeneration failure keeps the previous config
- **WHEN** a config regeneration with TUN enabled on sing-box cannot collect the connected-network prefixes
- **THEN** the previously generated config file SHALL be kept, and the failure SHALL be reported in the log and as a notification

### Requirement: Failed recovery remains pending
The system SHALL retain the TUN recovery marker when cleanup fails or times out and SHALL report the failure. Cleanup SHALL propagate enumeration and mutation failures while attempting independent cleanup operations.
#### Scenario: Recovery retry
- **WHEN** cleanup fails and the application restarts
- **THEN** recovery SHALL be attempted again and the marker SHALL clear only after successful cleanup

### Requirement: Journal identity gates helper commands
Automatic xray helper operations SHALL require a versioned journal and a separately supplied matching session identity. Broad legacy cleanup SHALL require an explicit operator override, never selected automatically by the desktop.

#### Scenario: Delayed stop observes a replacement journal
- **WHEN** a stop carries a session identity different from the current journal
- **THEN** the helper SHALL refuse without mutating kernel state or the journal

### Requirement: Namespace reservation prevents session collisions
The helper SHALL serialize mutations per network namespace and reserve xray routing state before backend spawn. It SHALL refuse occupied tables, reserved rule priorities, or interface names. A successful reservation SHALL bind its kernel index and session alias to the journal. Missing or mismatched reservation evidence SHALL refuse automatic mutation.

#### Scenario: Another profile owns the routing namespace
- **WHEN** another session already owns the kernel reservation
- **THEN** setup SHALL fail without changing that session's routes, rules, device, or journal

#### Scenario: Reservation tagging was interrupted
- **WHEN** a reservation exists without the recorded session alias
- **THEN** automatic recovery SHALL refuse and retain the journal

### Requirement: Cleanup verifies exact ownership
Cleanup SHALL verify the reservation, device identity, and exact recorded route/rule objects, including multiplicity, before deleting anything. Unknown effects and foreign state SHALL retain recovery state and refuse automatic mutation. Intent SHALL be written before addition and the observed object recorded after acknowledgement.

#### Scenario: Foreign state occupies a reserved priority
- **WHEN** foreign or duplicate rule objects appear at a reserved priority
- **THEN** automatic setup or cleanup SHALL refuse before deleting any resource

#### Scenario: Device name or index is reused
- **WHEN** the live device alias or index differs from the recorded identity
- **THEN** automatic teardown SHALL refuse and retain the journal

#### Scenario: Kernel acknowledgement was not recorded
- **WHEN** a mutation intent remains pending and live state contains an unrecorded object
- **THEN** automatic recovery SHALL refuse without treating the object as owned

#### Scenario: Recovery repeats after verified completion
- **WHEN** a completed session is recovered again
- **THEN** the helper SHALL succeed without changing current networking

### Requirement: Unverified sing-box recovery requires an operator
Sing-box startup SHALL remain helper-free. Automatic destructive recovery SHALL be refused when ownership of its self-managed state cannot be established.

#### Scenario: Sing-box session ended without verified cleanup
- **WHEN** a previous sing-box session has pending recovery without ownership evidence
- **THEN** the desktop SHALL retain its journal and request operator recovery without flushing tables or deleting a live device

### Requirement: Sing-box clean stop requires verified absence
Before reporting a clean sing-box TUN stop, the system SHALL verify that the device, configured table 2022, and rules selecting that table are absent. Inspection SHALL require no privileges and SHALL NOT delete resources. Failure or remaining state SHALL retain the journal and report a cleanup error. Launch intent SHALL be recorded before spawn.

#### Scenario: Foreign or leftover sing-box state remains
- **WHEN** a device, route, or selecting rule remains during stop verification
- **THEN** the system SHALL preserve it, retain the journal, and report pending recovery

#### Scenario: Sing-box never started or already completed
- **WHEN** a valid journal proves the session is unstarted Prepared or verified Complete
- **THEN** the record MAY be retired without invoking destructive helper recovery

### Requirement: Strict IPv6 TUN captures port-53 DNS without an IPv6 tunnel address
When `xray-up` runs with `--strict` and `--capture-dns` on a host with IPv6 available, the helper SHALL install the IPv6 port-53 DNS capture rules (pref 8999) into table 2023 even when no IPv6 tunnel address is configured, so unmarked queries to on-link IPv6 resolvers hit the strict unreachable route instead of leaking on the local link.

#### Scenario: On-link IPv6 DNS blocked under IPv4-only strict xray TUN
- **WHEN** `xray-up` runs with `--strict` and `--capture-dns`, no IPv6 tunnel address is supplied, and the host has an on-link IPv6 resolver (`fe80::1` or a `/64` LAN address)
- **THEN** unmarked IPv6 UDP and TCP packets to port 53 SHALL match the pref 8999 rule into table 2023 and hit the strict `unreachable` route rather than leaving on the LAN link

#### Scenario: Host with IPv6 disabled skips IPv6 DNS capture
- **WHEN** `xray-up` runs with `--strict` and `--capture-dns` on a host with IPv6 disabled
- **THEN** no IPv6 port-53 capture rule SHALL be installed and IPv4 setup SHALL succeed

### Requirement: Legacy helper override requires root
The route helper's `--force-legacy` flag SHALL require effective user ID 0. Invocations with `--force-legacy` by non-root accounts holding only file capabilities via the `v2ray-rs` group SHALL fail before opening netlink or mutating kernel state.

#### Scenario: Unprivileged group member rejected on force-legacy
- **WHEN** a non-root user in the `v2ray-rs` group invokes `v2ray-rs-netctl --force-legacy recover --xray` directly
- **THEN** the helper SHALL exit with a permission error and SHALL make no netlink or journal changes

#### Scenario: Root operator recovery allowed on force-legacy
- **WHEN** an operator invokes `v2ray-rs-netctl --force-legacy recover --xray` with effective UID 0
- **THEN** the helper SHALL execute the legacy cleanup pass

### Requirement: Parked TUN state tears down before incompatible connect and drops failed managers
Before starting a non-TUN or differently-configured TUN connection, or when a follow-up connect from terminal `Error` aborts in preflight, the system SHALL tear down any parked fail-closed TUN routing state and journal first. Once a replacement TUN candidate adopts the active TUN session and reaches `Running`, parked failed candidate `ProcessManager`s SHALL drop immediately without running tunnel teardown or stop checks against the shared interface and journal.

#### Scenario: Switching from strict TUN Error to non-TUN Connect clears routes first
- **WHEN** strict `xray` TUN is in `Error` with fail-closed routes parked, the user disables TUN or switches to `v2ray` (including via Apply & Restart), and initiates Connect
- **THEN** the system SHALL tear down the parked `xray` TUN routes and journal before starting the non-TUN connection

#### Scenario: Preflight abort on pending reconnect from TUN Error releases routes
- **WHEN** strict `xray` TUN enters terminal `Error` with a pending reconnect, deferred connect, or direct-node connect queued, and that follow-up connect aborts in preflight (no enabled nodes, disabled target node, missing backend binary, disabled host IPv6 with an IPv6 tunnel address, routing load error, or missing geodata)
- **THEN** the system SHALL release the parked TUN routing state and clear the completed recovery journal instead of leaving fail-closed routes installed while disconnected

#### Scenario: Compatible TUN candidate failover preserves the replacement tunnel
- **WHEN** candidate 1 fails before or after TUN device creation under `xray` or `sing-box` TUN and candidate 2 on the same backend and interface reaches `Running`
- **THEN** dropping candidate 1's parked state SHALL NOT invoke `xray-down`, delete candidate 2's live TUN device or policy rules, complete candidate 2's active journal, or run sing-box absence checks against candidate 2's live interface

#### Scenario: Cleanup fails before an incompatible connect
- **WHEN** teardown of the parked session fails or cannot verify ownership
- **THEN** the system SHALL retain the journal, report the failure, and leave the requested backend unstarted

### Requirement: xray route setup reconciles optional policy rules across re-runs
Each `xray-up` run on an owned TUN session SHALL reconcile optional UID-bypass (pref 8998) and port-53 DNS-capture (pref 8999) policy rules to match the current invocation flags, removing any journaled rule at those priorities whose flag is absent on the current run.

#### Scenario: Failover from pinned to unpinned candidate removes DNS capture rules
- **WHEN** candidate 1 ran `xray-up` with `--capture-dns` and failed, and candidate 2 on the same TUN session runs `xray-up` without `--capture-dns`
- **THEN** the journaled pref 8999 UDP and TCP port-53 capture rules from candidate 1 SHALL be removed and unmarked DNS queries SHALL no longer be steered into table 2023 by pref 8999

#### Scenario: Re-running xray-up without bypass UID removes prior bypass rule
- **WHEN** an owned session previously installed a pref 8998 UID-range rule and `xray-up` runs again without `--bypass-uid`
- **THEN** the prior pref 8998 rule SHALL be removed from both kernel policy state and the ownership journal
