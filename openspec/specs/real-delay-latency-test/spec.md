## ADDED Requirements

### Requirement: On-demand real-delay probe per node
The system SHALL provide a user-triggered "Real Delay" latency probe that measures the end-to-end wall-clock time of an HTTP request issued through each tested proxy node, including the proxy protocol handshake, TLS negotiation, and HTTP round trip. The system SHALL NOT implement any proxy protocol in v2ray-rs itself; it SHALL delegate the dial through the installed backend binary.

#### Scenario: User runs Real Delay on a selection of nodes
- **WHEN** the user invokes the "Test Real Delay" action on one or more enabled subscription nodes or manual nodes
- **THEN** the system SHALL produce, for each tested node, either a duration in milliseconds or a failure indicator, and SHALL surface the result to the UI without blocking other user actions

#### Scenario: Real Delay does not disturb the active connection
- **WHEN** the user invokes "Test Real Delay" while the user-facing backend process is running
- **THEN** the system SHALL NOT stop, restart, or reconfigure the user-facing backend process, and the active connection SHALL remain established

### Requirement: Ephemeral isolated backend for probes
The system SHALL run the Real Delay probe in an isolated, short-lived backend instance that is separate from the user-facing process. The ephemeral instance SHALL bind its API only to a loopback address on an OS-assigned port, SHALL NOT expose any user-facing proxy inbound, SHALL NOT write a persistent PID file, SHALL NOT publish process state to the tray or main UI log buffer, and SHALL be terminated at the end of the probe session.

#### Scenario: Probe session lifecycle
- **WHEN** a probe session starts
- **THEN** the system SHALL generate a probe config in a temporary file, spawn the backend with it, wait for the backend's API to become reachable on its loopback port within 2 seconds, run the probes, then stop the backend (SIGTERM, followed by SIGKILL after 5 seconds if needed) and remove the temporary config

#### Scenario: Crash recovery is disabled for probes
- **WHEN** the ephemeral probe backend exits unexpectedly during a session
- **THEN** the system SHALL report the in-flight probes as failures and SHALL NOT automatically restart the ephemeral instance

#### Scenario: Drop kills orphan probe
- **WHEN** the probe runner is dropped (caller cancels, panic, app shutdown) while the ephemeral backend is still alive
- **THEN** the system SHALL send SIGKILL to the child process before the runner is released

#### Scenario: API bound only to loopback
- **WHEN** the probe runner generates the ephemeral config
- **THEN** the API listener SHALL be configured to bind to `127.0.0.1` on a port obtained by binding `127.0.0.1:0` and immediately closing the socket, and SHALL NOT be reachable from any non-loopback interface

### Requirement: Bulk probe per session
The system SHALL test all selected nodes through a single ephemeral backend instance per session. The system SHALL NOT spawn one backend per node.

#### Scenario: Multiple nodes in one session
- **WHEN** the user selects N enabled nodes for Real Delay testing
- **THEN** the system SHALL include all N nodes as outbounds in one generated probe config, start one ephemeral backend, drive N parallel HTTP delay queries via the backend's API, and collect all results before terminating the backend

#### Scenario: Single-session concurrency limit
- **WHEN** a Real Delay session is already running and the user invokes "Test Real Delay" again
- **THEN** the system SHALL either queue the new request behind the in-flight session or reject it with a user-visible toast; it SHALL NOT spawn a second ephemeral backend in parallel

### Requirement: Backend-specific probe transport
The system SHALL use the installed backend's native delay-test surface to perform the HTTP probe:
- For `sing-box` it SHALL use the Clash-compatible API (`GET /proxies/{tag}/delay?url=<test_url>&timeout=<ms>`) and require the binary to expose `clash_api`.
- For `xray` (and v2ray-core ≥ 5) it SHALL use `burstObservatory` (or `observatory` as a fallback) declared in the probe config and read results from the backend's API.

#### Scenario: sing-box probe round trip
- **WHEN** the installed backend is sing-box with `clash_api` available and a probe session runs against a node tagged `node-3`
- **THEN** the system SHALL issue `GET http://127.0.0.1:<port>/proxies/node-3/delay?url=<configured_url>&timeout=<configured_ms>` and SHALL parse the returned `delay` field (in ms) as the result

#### Scenario: xray probe round trip
- **WHEN** the installed backend is xray and a probe session runs
- **THEN** the system SHALL declare `burstObservatory` (or `observatory`) referencing the probed outbound tags with `probeUrl` set to the configured URL, wait at least one probe interval plus the configured timeout, then fetch the observation report from the backend's API and map each tag's recorded delay to the corresponding node

### Requirement: Graceful degradation when backend lacks support
The system SHALL detect whether the installed backend exposes the required delay-test surface and SHALL disable the Real Delay action with a user-visible explanation when it does not.

#### Scenario: sing-box without clash_api
- **WHEN** the installed sing-box binary is built without the `with_clash_api` tag and the user invokes "Test Real Delay"
- **THEN** the system SHALL surface a toast explaining that the binary lacks Clash API support and SHALL link to documentation, and SHALL keep the TCP-ping action available

#### Scenario: v2ray-core legacy without observatory
- **WHEN** the installed backend is a v2ray-core build without observatory support
- **THEN** the system SHALL hide or disable the Real Delay action for that backend and SHALL show "Real Delay not supported by this backend" in the affected menus

### Requirement: Probe test URL and timeout are configurable
The system SHALL accept a user-configured test URL and per-probe timeout. The system SHALL apply sane defaults: test URL `https://www.gstatic.com/generate_204`, timeout 5000 ms. The system SHALL validate the URL as a syntactically valid `http://` or `https://` URL before accepting it.

#### Scenario: Default Real Delay settings
- **WHEN** the user has never modified Real Delay settings
- **THEN** the test URL SHALL be `https://www.gstatic.com/generate_204` and the timeout SHALL be `5000` ms

#### Scenario: User changes test URL
- **WHEN** the user enters `https://cp.cloudflare.com/generate_204` and saves settings
- **THEN** subsequent Real Delay sessions SHALL use that URL and the value SHALL persist across app restarts

#### Scenario: Invalid URL rejected
- **WHEN** the user enters `not-a-url` or `ftp://example.com/`
- **THEN** the system SHALL reject the input with an inline validation error and SHALL NOT persist the change

### Requirement: Per-node Real Delay result persistence
The system SHALL persist the most recent Real Delay sample per subscription node and per manual node in the latency snapshot file alongside the existing TCP sample, keyed by stable node identity. A missing or stale sample SHALL be representable as "unknown" without conflating it with a successful zero-ms result.

#### Scenario: Sample is written after a successful probe
- **WHEN** a Real Delay probe returns a duration for a given node
- **THEN** the system SHALL update that node's `last_real_delay_ms` to the duration and SHALL persist the change atomically to the latency snapshot file

#### Scenario: Sample is cleared on probe failure
- **WHEN** a Real Delay probe times out or the backend reports an error for a given node
- **THEN** the system SHALL record the failure (either by clearing `last_real_delay_ms` to `None` or by leaving the previous successful sample intact, per implementation choice documented in the design), and SHALL NOT record a misleading zero-ms result

#### Scenario: Startup hydration of Real Delay
- **WHEN** the app launches and the latency snapshot file contains `last_real_delay_ms` values
- **THEN** each affected node SHALL display its previously recorded Real Delay value in the subscriptions UI without requiring a fresh probe
