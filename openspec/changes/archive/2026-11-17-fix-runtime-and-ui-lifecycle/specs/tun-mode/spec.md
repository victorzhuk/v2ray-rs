# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: Parked TUN state tears down before incompatible connect and drops failed managers
Before starting a non-TUN connection or a TUN connection whose backend or interface differs from a parked fail-closed TUN session, the system SHALL tear down the parked TUN routing state and journal first. Once a failover candidate reaches `Running`, any parked failed candidate `ProcessManager`s from earlier attempts SHALL be dropped immediately.

#### Scenario: Switching from strict TUN Error to non-TUN Connect clears routes first
- **WHEN** strict `xray` TUN is in `Error` with fail-closed routes parked, the user disables TUN, and clicks Connect
- **THEN** the system SHALL tear down the parked `xray` TUN routes and journal before starting the non-TUN connection

#### Scenario: Cleanup fails before an incompatible connect
- **WHEN** teardown of the parked session fails or cannot verify ownership
- **THEN** the system SHALL retain the journal, report the failure, and leave the requested backend unstarted
