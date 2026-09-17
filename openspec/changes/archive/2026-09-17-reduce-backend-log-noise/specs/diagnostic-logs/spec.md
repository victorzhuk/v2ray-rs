## ADDED Requirements

### Requirement: Backend log verbosity is user-controlled
The System page of Preferences SHALL offer a backend log level selector (`error`, `warning`, `info`, `debug`) and a connection log switch. The connection log switch SHALL be insensitive while the selected backend is sing-box, with a note that sing-box logs connections at the `info` and `debug` levels.

#### Scenario: Controls shown
- **WHEN** the user opens the System page with the xray backend selected
- **THEN** a backend log level selector showing `warning` and an off connection log switch SHALL be visible and sensitive

#### Scenario: sing-box note
- **WHEN** the selected backend is sing-box
- **THEN** the connection log switch SHALL be insensitive and its note SHALL say connection lines appear at `info` and `debug`

### Requirement: Backend lines are written without terminal escapes
The system SHALL remove ANSI escape sequences from every backend and route-helper line before writing it to the backend log file or showing it on the logs page. Record timestamps written by the backend SHALL be kept unchanged, and each session record SHALL state the local UTC offset in effect when it was written.

#### Scenario: sing-box color codes
- **WHEN** sing-box writes `\x1b[31mFATAL\x1b[0m[0000] start service: …`
- **THEN** `backend.log` SHALL contain `FATAL[0000] start service: …` with no escape bytes

#### Scenario: UTC offset recorded
- **WHEN** a session starts on a host whose local time zone is UTC+03:00
- **THEN** its session record SHALL contain `utc_offset=+03:00`

### Requirement: Backend deprecation and security warnings are surfaced
The system SHALL show a non-blocking toast for the first backend line in a connection that matches a known deprecation or security warning pattern, quoting the matched warning. Each pattern SHALL toast at most once per connection. The patterns SHALL include, for xray, lines reporting a deprecated feature and the REALITY `received real certificate (potential MITM or redirection)` error, and for sing-box, `WARN` lines reporting a deprecated feature.

#### Scenario: xray WebSocket deprecation
- **WHEN** xray writes `[Warning] common/errors: The feature WebSocket transport (with ALPN http/1.1, etc.) is deprecated, not recommended for using and might be removed. Please migrate to XHTTP H2 & H3 as soon as possible.` during a connection
- **THEN** one toast SHALL quote that the WebSocket transport is deprecated

#### Scenario: Repeated REALITY warning
- **WHEN** xray writes the REALITY `potential MITM or redirection` error three times within one connection
- **THEN** exactly one toast SHALL be shown for it

#### Scenario: Warnings are never access noise
- **WHEN** the connection log is off
- **THEN** warning and error lines SHALL still reach `backend.log` and the logs page
