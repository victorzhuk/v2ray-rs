## MODIFIED Requirements

### Requirement: Capture launched runtime snapshot
The system SHALL capture an immutable snapshot of the restart-relevant settings and routing rules that were actually used for the current connection attempt. Restart-relevant settings SHALL include every setting that changes the generated backend config, including the TUN settings and the connection idle-timeout and WebSocket-heartbeat settings. While connected, UI state derived from those settings SHALL follow the snapshot rather than the edited settings.

#### Scenario: Snapshot captured before launch
- **WHEN** the app prepares the config inputs for `Connect`
- **THEN** it stores the exact settings and routing rules passed to config generation before backend start begins

#### Scenario: TUN indicators follow the running session
- **WHEN** TUN is enabled in settings while a session launched without TUN is running
- **THEN** the app SHALL keep treating the session as non-TUN — background latency probes continue and no TUN recovery marker is written — until the session is restarted with the new settings

### Requirement: Apply pending runtime changes by restart
The system SHALL apply pending runtime configuration changes by reusing the normal disconnect/reconnect flow. When the running session was started by a direct connection to a chosen node, the reconnect SHALL target that node as the sole candidate.

#### Scenario: Apply and restart while connected
- **WHEN** the user chooses "Apply & Restart" from the restart-required banner
- **THEN** the system disconnects, reconnects with the already-persisted runtime config, and replaces the active runtime snapshot with the new launched snapshot

#### Scenario: Apply and restart keeps a directly chosen node
- **WHEN** the running session was started by a direct connection to a node and the user chooses "Apply & Restart"
- **THEN** the system SHALL reconnect to that same node as the sole candidate

#### Scenario: Directly chosen node no longer available
- **WHEN** the user chooses "Apply & Restart" and the directly chosen node has been removed or disabled
- **THEN** the system SHALL notify the user and reconnect using the configured strategy

#### Scenario: TUN edit raises the restart banner
- **WHEN** the user changes any TUN setting, the idle timeout, or the WebSocket heartbeat while connected
- **THEN** the restart-required banner SHALL appear
