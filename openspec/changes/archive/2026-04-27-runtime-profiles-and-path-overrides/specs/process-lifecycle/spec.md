# Spec Delta: process-lifecycle

## MODIFIED Requirements

### Requirement: Cleanup on app exit
The system SHALL ensure the backend process is terminated when the application exits, using a profile-scoped PID file.

#### Scenario: Normal app exit
- **WHEN** the user quits the application
- **THEN** the system SHALL send SIGTERM to the backend process and wait for it to exit before completing shutdown

#### Scenario: PID file for crash recovery
- **WHEN** the app starts and finds a PID file from a previous run at `runtime_dir/backend.pid`
- **THEN** the system SHALL check if that process is still running and kill it if so

#### Scenario: PID file does not leak across profiles
- **WHEN** the app launches with one profile while a backend from a different profile is running
- **THEN** the system SHALL only inspect the PID file under the active profile's `runtime_dir` and SHALL NOT touch other profiles' processes

## ADDED Requirements

### Requirement: Single-instance lock per profile
The system SHALL acquire an exclusive advisory lock on `runtime_dir/v2ray-rs.lock` at startup, before initializing persistence or spawning the backend, and SHALL hold it for the lifetime of the process. Two instances of the same profile SHALL NOT run concurrently. Two instances of different profiles SHALL be able to run concurrently because their `runtime_dir`s differ.

#### Scenario: Second instance of the same profile is refused
- **WHEN** an instance is already running for a given profile and a second invocation targets the same profile
- **THEN** the second invocation SHALL fail to acquire the lock, SHALL print the holder PID recorded in `instance.json`, and SHALL exit with code 75

#### Scenario: Different profiles run side-by-side
- **WHEN** an instance is running with `--profile production` and another is started with `--profile development`
- **THEN** both instances SHALL run concurrently without contending for the same lock file

#### Scenario: Lock is released on shutdown
- **WHEN** an instance exits cleanly or is killed
- **THEN** the kernel SHALL release the advisory lock and a subsequent invocation of the same profile SHALL be able to acquire it
