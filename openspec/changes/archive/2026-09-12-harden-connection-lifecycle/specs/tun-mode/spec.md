## MODIFIED Requirements

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

## ADDED Requirements

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
