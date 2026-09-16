## MODIFIED Requirements

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
