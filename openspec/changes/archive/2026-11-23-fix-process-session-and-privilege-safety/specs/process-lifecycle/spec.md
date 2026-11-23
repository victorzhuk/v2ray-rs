# Spec Delta: process-lifecycle

## ADDED Requirements

### Requirement: Lossless advisory channel compiles and delivers to subscribers
`ProcessManager::subscribe_advisories` SHALL return a lossless `mpsc::UnboundedReceiver<ProcessAdvisory>` backed by multi-subscriber fan-out, delivering engine degradation notices even when the log broadcast lags.

#### Scenario: Advisory arrives while log broadcast lags
- **WHEN** rapid backend output causes `subscribe_logs()` to return `Lagged` and the anti-DPI engine exits unexpectedly
- **THEN** every active `subscribe_advisories()` receiver SHALL receive `ProcessAdvisory::AntidpiDegraded` exactly once

### Requirement: Failover preserves parked TUN managers and drains orphaned cleanups
On candidate failure, crash give-up, `Stop`, or `prepare_tun_session` error, the session supervisor SHALL keep the active `parked` manager until superseded or stopped and SHALL always drain `orphaned` managers before reporting `Stopped` or terminal `Error`.

#### Scenario: Candidate 1 stays parked across candidate 2 failover
- **WHEN** candidate 1 fails with an active TUN kill-switch and candidate 2 begins starting
- **THEN** candidate 1 SHALL remain in `parked` (`probe_allowed` stays `false`) until candidate 2 adopts the same TUN identity or teardown releases candidate 1

#### Scenario: Orphaned cleanup failure blocks Stopped even when active manager stops cleanly
- **WHEN** a superseded manager sits in `orphaned` and the user clicks Stop while the active manager stops cleanly
- **THEN** the supervisor SHALL drain `orphaned` into `retry_cleanup` rather than emitting `ProcessState::Stopped`

#### Scenario: Preflight cancellation cleans up a prepared Xray TUN journal
- **WHEN** `Stop` cancels a start after `prepare_tun_session` wrote `Phase::Prepared`
- **THEN** teardown SHALL clear or complete the prepared journal so later connections and backend switches are not blocked

### Requirement: Session stop is non-blocking and PID file writes are race-free
`SessionHandle::stop` SHALL never block the calling thread or panic inside a Tokio runtime, `PidFile::write` SHALL use a unique temp file per write, and blocking NSS or `fsync` operations in async paths SHALL run via `spawn_blocking`.

#### Scenario: Repeated stop calls never block or panic
- **WHEN** `SessionHandle::stop` is invoked more than 4 times from a Tokio worker or GTK thread while teardown is in flight
- **THEN** every call SHALL return immediately without blocking or panicking

#### Scenario: Concurrent PID writes never delete each other's temp file
- **WHEN** two tasks in the same process invoke `PidFile::write` concurrently
- **THEN** each writer SHALL use a distinct temp file via `NamedTempFile::new_in` and leave a valid 0600 PID file
