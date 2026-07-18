## MODIFIED Requirements

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
