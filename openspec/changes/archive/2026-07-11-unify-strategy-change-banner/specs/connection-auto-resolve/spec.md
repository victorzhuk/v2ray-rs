## ADDED Requirements

### Requirement: Strategy changes take effect on the next connection
A change to the auto-resolve strategy SHALL take effect at the next connection attempt. While a connection is active, the system SHALL NOT automatically disconnect to apply a strategy change; the running session continues under the strategy it was started with until the user explicitly applies the change or reconnects.

#### Scenario: Active session keeps its strategy
- **WHEN** the user changes the strategy while connected and does not apply the restart
- **THEN** the active session SHALL continue unchanged and its displayed connection metadata SHALL keep reporting the strategy it was started with

#### Scenario: Next connect uses the new strategy
- **WHEN** a new connection starts after a strategy change (explicit apply, manual reconnect, or a later connect)
- **THEN** candidate ordering SHALL follow the new strategy
