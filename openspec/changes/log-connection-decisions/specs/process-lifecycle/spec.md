## ADDED Requirements

### Requirement: Termination signals and panics leave a record
The application SHALL handle SIGTERM, SIGINT, and SIGHUP by running the same quit path as a user quit, so a running backend is stopped, its exit record is written with reason `app-quit`, and TUN state is released before the process exits. A panic in the application SHALL be written to the application log with its message and location before the default panic behavior continues.

#### Scenario: SIGTERM while connected
- **WHEN** the application receives SIGTERM while a backend is running
- **THEN** the app log SHALL record the signal, the backend SHALL be stopped gracefully, and `backend.log` SHALL contain an exit record with reason `app-quit`

#### Scenario: Panic is logged
- **WHEN** code on any thread panics
- **THEN** the app log file SHALL contain an `error` record with the panic message and source location
