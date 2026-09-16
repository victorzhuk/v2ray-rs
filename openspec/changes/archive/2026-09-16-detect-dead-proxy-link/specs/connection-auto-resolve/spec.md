## ADDED Requirements

### Requirement: Opt-in failover of an unhealthy session
The system SHALL provide a preference, off by default, to fail over a session that becomes unhealthy. When it is on and a session started by the configured strategy becomes unhealthy, the system SHALL disconnect and reconnect using the configured strategy with the unhealthy node excluded from that attempt's candidates. The system SHALL perform at most 3 consecutive health failovers without a successful probe in between; a successful probe or a user-initiated Connect, direct connect, or Disconnect SHALL reset that count. A session started by a direct connection to a chosen node SHALL NOT fail over for health, regardless of the preference. A user-initiated Connect, direct connect, or Disconnect SHALL cancel a pending health failover.

#### Scenario: Failover off by default
- **WHEN** the preference has never been changed and a strategy-planned session becomes unhealthy
- **THEN** the session SHALL keep running on the same node

#### Scenario: Failover excludes the dead node
- **WHEN** the preference is on and a strategy-planned session on node A becomes unhealthy
- **THEN** the system SHALL reconnect with the configured strategy and node A SHALL NOT be a candidate for that attempt

#### Scenario: Direct connection never fails over
- **WHEN** the preference is on and a session started by a direct connection to node A becomes unhealthy
- **THEN** the session SHALL keep running on node A and only the unhealthy status SHALL be shown

#### Scenario: Budget bounds repeated failovers
- **WHEN** the preference is on and three consecutive health failovers each end in an unhealthy session without any successful probe
- **THEN** the system SHALL NOT fail over again and SHALL keep the last session running with the unhealthy status
