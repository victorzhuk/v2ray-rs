## Purpose

Detects that a running connection no longer carries traffic, by probing the proxy path through the session's own local inbound, and tells the user instead of reporting a dead session as Connected.

## ADDED Requirements

### Requirement: Running connection is probed through its own inbound
While a connection is `Running` and health checks are enabled (the default), the system SHALL request the configured Real Delay test URL through the session's local HTTP inbound, first 10 seconds after the connection is reported `Running` and then every 30 seconds, each request bounded by the configured Real Delay timeout. A response with a status below 400 SHALL count as a success; a connection, TLS, or protocol error or a timeout SHALL count as a failure. After 3 consecutive failures the session SHALL be unhealthy; the next success SHALL make it healthy. Probing SHALL pause while the backend is being respawned and restart from a healthy state when it is `Running` again. Health SHALL NOT change the process state. When health checks are disabled, no probe SHALL be sent.

#### Scenario: Dead proxy path becomes unhealthy
- **WHEN** a session is `Running` and three consecutive probes fail with a TLS certificate error
- **THEN** the session SHALL be unhealthy while its process state remains `Running`

#### Scenario: Recovery
- **WHEN** an unhealthy session's next probe gets a `204` response
- **THEN** the session SHALL be healthy

#### Scenario: Isolated failure
- **WHEN** one probe fails and the next succeeds
- **THEN** the session SHALL stay healthy

#### Scenario: Respawn resets health
- **WHEN** the backend crashes and is respawned while the session is unhealthy
- **THEN** no probe SHALL be sent until the respawned backend is `Running`, and the session SHALL then be healthy until three new consecutive failures

#### Scenario: Checks disabled
- **WHEN** health checks are disabled in preferences
- **THEN** no probe SHALL be sent and the session SHALL never be marked unhealthy

### Requirement: Unhealthy session is announced once per streak
When a session becomes unhealthy, the system SHALL show a notification in the main window stating that the proxy is not responding and naming the last probe failure, and, when desktop notifications are enabled, SHALL send a desktop notification with the same content. The system SHALL NOT repeat either while the session stays unhealthy.

#### Scenario: First transition notifies
- **WHEN** a healthy session becomes unhealthy and desktop notifications are enabled
- **THEN** one in-window notification and one desktop notification SHALL be shown

#### Scenario: Continued failures stay quiet
- **WHEN** further probes fail while the session is already unhealthy
- **THEN** no new notification SHALL be shown

### Requirement: xray DNS through the proxy is flagged when failing
When the backend is xray, the system SHALL count backend log lines reporting `app/dns: failed to retrieve response` and SHALL mark DNS through the proxy as failing when at least 20 such lines occur within 60 seconds; the mark SHALL clear after 60 seconds without such a line. Log lines that carry no destination, such as `proxy/tun: connection reset by peer` and `proxy/tun: connection was refused`, SHALL NOT affect health. The DNS mark SHALL NOT make the session unhealthy. Other backends SHALL NOT be classified from their log output.

#### Scenario: Sustained DNS failures
- **WHEN** an xray session logs 30 `app/dns: failed to retrieve response` lines within one minute
- **THEN** DNS through the proxy SHALL be marked failing

#### Scenario: Tunnel noise ignored
- **WHEN** an xray session logs 500 `proxy/tun: connection reset by peer` lines within one minute and probes succeed
- **THEN** the session SHALL stay healthy and DNS SHALL NOT be marked failing

#### Scenario: Mark clears
- **WHEN** DNS is marked failing and 60 seconds pass without an `app/dns: failed to retrieve response` line
- **THEN** the mark SHALL clear
