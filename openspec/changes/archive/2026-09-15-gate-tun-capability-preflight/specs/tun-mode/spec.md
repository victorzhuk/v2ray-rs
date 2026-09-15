## MODIFIED Requirements

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
