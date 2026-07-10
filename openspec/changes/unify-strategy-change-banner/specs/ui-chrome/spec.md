## ADDED Requirements

### Requirement: Restart-required banner for strategy changes
Changing the auto-resolve strategy (or the Real Delay ranking toggle) while the backend is starting or running SHALL NOT disconnect automatically; it SHALL mark the runtime configuration as diverged and reuse the restart-required banner, applying the change only on explicit "Apply & Restart".

#### Scenario: Strategy change while connected shows the banner
- **WHEN** the backend is connected and the user changes the auto-resolve strategy
- **THEN** the connection SHALL stay up and the restart-required banner SHALL appear with `Apply & Restart`

#### Scenario: Strategy change applies on explicit restart
- **WHEN** the user clicks `Apply & Restart` after a strategy change
- **THEN** the system SHALL disconnect and reconnect using the new strategy

#### Scenario: Strategy change while disconnected applies silently
- **WHEN** the backend is stopped and the user changes the auto-resolve strategy
- **THEN** no banner SHALL appear and the next connection SHALL use the new strategy
