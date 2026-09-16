## ADDED Requirements

### Requirement: Status bar distinguishes restarts from connects
While a connection is starting, the status bar SHALL distinguish why: an in-place respawn after the backend exited unexpectedly SHALL show "Restarting after crash", and an attempt started by auto-reconnect SHALL show "Reconnecting (n/3)" with the current attempt number. Other starts SHALL keep showing "Connecting…".

#### Scenario: Crash respawn
- **WHEN** a connected backend exits unexpectedly and the system respawns it
- **THEN** the status bar SHALL show "Restarting after crash" until the backend is running again or the connection ends

#### Scenario: Auto-reconnect attempt
- **WHEN** the second scheduled auto-reconnect starts a connection
- **THEN** the status bar SHALL show "Reconnecting (2/3)"

#### Scenario: User connect
- **WHEN** the user clicks Connect
- **THEN** the status bar SHALL show "Connecting…"
