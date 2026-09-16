## MODIFIED Requirements

### Requirement: Connection terminal state has a single source
The system SHALL report a connection's terminal state (`Stopped` or `Error`) only from the component supervising the whole connection attempt, never from an individual candidate's backend. A candidate given up during failover SHALL NOT be reported as the connection stopping. When two consecutive candidates of the same connection attempt fail with the same reason — compared after removing terminal color codes, timestamps and elapsed-time counters, and each candidate's own name, address, and port — the system SHALL stop failing over and report a single `Error` stating that the same error repeated on consecutive nodes and is not node-specific, followed by that reason. Failure text reported to the user SHALL NOT contain terminal color escape sequences.

#### Scenario: Failover is not reported as a stop
- **WHEN** a candidate exhausts its crash budget and the connection fails over to the next candidate
- **THEN** the app SHALL NOT receive `Stopped` for the connection, SHALL keep its connection handle, and SHALL keep the TUN recovery marker

#### Scenario: Disconnect works after failover
- **WHEN** the connection has failed over to another candidate and reached `Running`
- **THEN** Disconnect SHALL stop that backend and report `Stopped`

#### Scenario: Last candidate fails
- **WHEN** every candidate of a connection attempt has failed with differing reasons
- **THEN** the app SHALL receive exactly one terminal `Error` summarizing the failures

#### Scenario: Same failure on consecutive candidates stops failover
- **WHEN** a connection attempt has seven candidates and the first two both fail with `FATAL[0000] start service: post-start inbound/tun[tun-in]: starting TUN interface: set rules: add rule 0/9: address family not supported by protocol` (one printed as `[0001]`)
- **THEN** no third candidate SHALL be started, and the app SHALL receive exactly one terminal `Error` that names the shared reason once, says it is not node-specific, and contains no color escape sequences

#### Scenario: Node-specific failures keep failing over
- **WHEN** the first candidate fails with a TLS error naming its own server and the second fails with the same error naming a different server
- **THEN** the connection SHALL fail over to the third candidate
