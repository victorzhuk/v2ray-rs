## MODIFIED Requirements

### Requirement: TUN-aware connection start and stop
The system SHALL make connection start and stop TUN-aware. For xray with TUN
enabled, after spawning the backend the system SHALL wait for the TUN device to
appear, bounded by a timeout, then invoke the route helper to program the address
and routes before reporting `Running`; if the device does not appear or the helper
fails, the system SHALL stop the backend and transition to `Error`. When the
`v2ray-rs-bypass` user exists, the system SHALL resolve its UID and pass it to the
route helper so the per-UID bypass policy rule is installed; absence of the user
SHALL NOT block the connection. For sing-box with TUN enabled, the backend programs
its own routes via `auto_route` and the system SHALL NOT run the route helper. Stop
SHALL remain SIGTERM-first so the backend can tear down its own routes before any
SIGKILL.

#### Scenario: xray TUN start programs routes
- **WHEN** the user connects with xray and TUN enabled
- **THEN** the system SHALL spawn xray, wait for the TUN device, invoke the route helper to add the routes, and only then report `Running`

#### Scenario: xray TUN start installs the bypass rule
- **WHEN** the user connects with xray and TUN enabled and the `v2ray-rs-bypass` user exists
- **THEN** the system SHALL resolve that user's UID and pass it to the route helper so the per-UID bypass policy rule is installed; when the user does not exist the connection SHALL proceed without the rule

#### Scenario: xray TUN device never appears
- **WHEN** xray is spawned in TUN mode but the device does not appear within the timeout
- **THEN** the system SHALL stop the backend and transition to `Error`

#### Scenario: sing-box TUN start needs no helper
- **WHEN** the user connects with sing-box and TUN enabled
- **THEN** the system SHALL spawn sing-box and rely on its `auto_route` to program routes, without invoking the route helper

#### Scenario: Graceful stop preserves teardown
- **WHEN** the user disconnects from a TUN session
- **THEN** the system SHALL send SIGTERM first so the backend can remove its own routes (sing-box) or close its TUN fd so the kernel drops the routes (xray), escalating to SIGKILL only after the timeout, and for xray SHALL invoke the route-helper teardown as a safeguard
