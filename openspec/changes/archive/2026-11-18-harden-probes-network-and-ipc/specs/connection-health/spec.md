# Spec Delta

## MODIFIED Requirements

### Requirement: Running connection is probed through its own inbound
While a connection is `Running` and health checks are enabled (the default), the system SHALL request the configured Real Delay test URL through the session's local HTTP inbound, first 10 seconds after the connection is reported `Running` and then every 30 seconds, each request bounded by the configured Real Delay timeout. A `2xx` or `3xx` response status for an `http://` `GET` probe and a `2xx` status for an `https://` `CONNECT` probe SHALL count as a success; `1xx` interim responses, non-`2xx` `CONNECT` responses, `4xx`/`5xx` responses, connection, TLS, or protocol errors, and timeouts SHALL count as failures. After 3 consecutive failures the session SHALL be unhealthy; the next success SHALL make it healthy. Probing SHALL pause while the backend is being respawned and restart from a healthy state when it is `Running` again. Health SHALL NOT change the process state. When health checks are disabled, no probe SHALL be sent.

#### Scenario: Dead proxy path becomes unhealthy
- **WHEN** a session is `Running` and three consecutive probes fail with a TLS certificate error
- **THEN** the session SHALL be unhealthy while its process state remains `Running`

#### Scenario: Interim 1xx or redirected CONNECT counts as failure
- **WHEN** a session probe receives `HTTP/1.1 100 Continue` without a final response or receives `HTTP/1.1 302 Found` in reply to an `https://` `CONNECT` request
- **THEN** that probe attempt SHALL count as a failure rather than marking the tunnel healthy

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

## ADDED Requirements

### Requirement: Health probe validates target URLs and HTTP Host headers
Health probes SHALL reject target URLs containing embedded credentials, URL fragments, ASCII control characters, missing hosts, or malformed IPv6 bracket authorities before opening a proxy socket. For `http://` targets on a non-default port, the `GET` request SHALL include `:<port>` in the `Host` header.

#### Scenario: Non-default HTTP port included in Host header
- **WHEN** a health probe requests `http://probe.example.com:8080/generate_204` through the local HTTP inbound
- **THEN** the request SHALL send `Host: probe.example.com:8080`

#### Scenario: Malformed authority or fragment rejected before dialing
- **WHEN** a health probe is invoked with `https://user:pass@example.com/`, `https://[::1]extra/path`, or `https://example.com:443#frag`
- **THEN** the probe SHALL fail URL validation immediately without sending a malformed `CONNECT` or `GET` line
