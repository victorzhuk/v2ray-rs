## MODIFIED Requirements

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

## ADDED Requirements

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
