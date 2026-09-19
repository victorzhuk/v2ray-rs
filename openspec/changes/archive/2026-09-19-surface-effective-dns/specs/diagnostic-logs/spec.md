## ADDED Requirements

### Requirement: Effective DNS is recorded per launch
Each backend launch, including a crash respawn, SHALL be followed in the backend log file, directly after its session record, by one record per effective resolver stating its address, path, source, and scope, computed from the settings actually used for that launch, including an imported profile and connect-time host overrides. When the connection's resolvers include fallback resolvers, the system SHALL write one notice line per connection to the process log stream stating that fallback resolvers are in use and which setting changes that.

#### Scenario: Records follow the session record
- **WHEN** an xray TUN connection starts with DNS disabled
- **THEN** `backend.log` SHALL contain, after the `session` record, records naming `https://1.1.1.1/dns-query` and `https://8.8.8.8/dns-query` with path `proxy` and source `fallback`

#### Scenario: Fallback notice once per connection
- **WHEN** such a connection fails over across two candidates
- **THEN** the fallback notice SHALL appear exactly once in the process log stream

#### Scenario: No notice with user resolvers
- **WHEN** DNS is enabled with at least one unscoped server
- **THEN** no fallback notice SHALL be written
