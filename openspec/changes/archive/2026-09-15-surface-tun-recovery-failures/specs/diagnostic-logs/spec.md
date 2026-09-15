## MODIFIED Requirements

### Requirement: Backend output survives the application
The system SHALL append every line the backend writes to stdout or stderr, and every line the route helper writes — including during a TUN route-recovery pass run outside a connection — to a backend log file under the state directory. Each backend launch SHALL be preceded by a session record naming the backend, its version, the node, and whether TUN is on, and each backend exit SHALL be followed by an exit record stating the exit code or signal, whether the stop was requested, the number of crashes in the current window, and the last output line. A route-recovery pass SHALL additionally record its outcome (success, exit status, or timeout).

#### Scenario: Crash reason is readable after restart
- **WHEN** the backend crashes and the application is later restarted
- **THEN** `<state_dir>/logs/backend.log` SHALL still contain the backend's last output lines and an exit record marking the exit as unrequested

#### Scenario: Lines are not lost under load
- **WHEN** the backend writes lines faster than the user interface consumes them
- **THEN** every line SHALL still be written to the backend log file

#### Scenario: Recovery output is kept
- **WHEN** a TUN route-recovery pass runs at startup or after a session ended without a clean stop
- **THEN** every line the route helper printed and the pass's outcome SHALL appear in `<state_dir>/logs/backend.log`, and none SHALL be written only to the application's standard error
