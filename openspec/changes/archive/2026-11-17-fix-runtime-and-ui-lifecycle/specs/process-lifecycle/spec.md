# Spec Delta

## MODIFIED Requirements

### Requirement: Single-instance lock per profile
The system SHALL acquire an exclusive advisory lock on `runtime_dir/v2ray-rs.lock` without truncating existing lock/stamp files before lock acquisition, and SHALL hold it for the lifetime of the process. On contention with default profile paths and an active session D-Bus primary application, a second invocation SHALL focus the primary window and exit with code 0; when per-directory overrides are active or D-Bus activation is unavailable, it SHALL print the holder PID from `instance.json` without modifying `instance.json` and exit with code 75.

#### Scenario: Second instance on default paths focuses primary window
- **WHEN** an instance is running for a profile with default paths (no `--*-dir` or `V2RAY_RS_*_DIR` overrides) and a second invocation targets the same profile in the same graphical session
- **THEN** the second invocation SHALL activate the running `com.github.v2ray-rs[.qualifier]` application window over session D-Bus, leave `instance.json` untouched, and exit with code 0

#### Scenario: Second instance of the same profile is refused
- **WHEN** an instance is already running for a given profile and a second invocation targets the same profile with `--*-dir` directory overrides or when session D-Bus activation is unavailable
- **THEN** the second invocation SHALL fail to acquire the lock, SHALL print the holder PID recorded in `instance.json` without overwriting `instance.json`, and SHALL exit with code 75

#### Scenario: Different profiles run side-by-side
- **WHEN** an instance is running with `--profile production` and another is started with `--profile development`
- **THEN** both instances SHALL run concurrently without contending for the same lock file

#### Scenario: Lock is released on shutdown
- **WHEN** an instance exits cleanly or is killed
- **THEN** the kernel SHALL release the advisory lock and a subsequent invocation of the same profile SHALL be able to acquire it

## ADDED Requirements

### Requirement: Orphan PID verification handles setcap procfs restrictions and atomic writes
`PidFile` SHALL write `runtime_dir/backend.pid` atomically with `0600` permissions and record process start-time ticks, binary path, and config path. When `/proc/<pid>/exe` returns `EACCES` on a capability-bearing binary, orphan detection SHALL verify `(pid, start_time_ticks, /proc/<pid>/cmdline -c <config_path>, /proc/<pid>/comm)` before signaling, and spawned children SHALL set `stdin` to null and `kill_on_drop(true)`.

#### Scenario: Orphaned setcap backend under suid_dumpable=2 is terminated
- **WHEN** the app starts with a `backend.pid` from a crashed capability-bearing backend whose `/proc/<pid>/exe` returns `EACCES`, and the start ticks, config argument, and process name all match the recorded launch
- **THEN** orphan cleanup SHALL verify and terminate that backend process without signaling unrelated PIDs

#### Scenario: Process identity cannot be confirmed
- **WHEN** the executable link is inaccessible and any required fallback identity field differs or cannot be read
- **THEN** orphan cleanup SHALL send no signal to that PID

### Requirement: Runtime snapshot tracks imported profile state and output directory
The launched `RuntimeSnapshot` SHALL capture the active subscription's `imported_profile` and `use_imported_profile` state alongside `backend.config_output_dir`, and `ConfigWriter` SHALL NOT alter permissions on a user-configured external `config_output_dir`.

#### Scenario: Toggling imported profile while connected triggers restart banner
- **WHEN** a session is connected using a subscription node and the user toggles `use_imported_profile` on that subscription
- **THEN** the active session SHALL report pending restart changes until "Apply & Restart" or reconnect

#### Scenario: Custom config_output_dir permissions are preserved
- **WHEN** `backend.config_output_dir` points to a user-specified directory outside `runtime_dir`
- **THEN** writing the generated config SHALL create the file atomically without calling `chmod 0700` on the parent directory

### Requirement: Stopping transitions serialize connect and disconnect commands
While a session is in `ProcessState::Stopping`, new `Connect`, `ConnectToNode`, and `Disconnect` actions SHALL wait for `Stopped` (or coerce cleanly without starting a parallel session), and stopping SHALL clear any stale `pending_direct_target`.

#### Scenario: Connect requested while previous session is Stopping
- **WHEN** the user triggers `Connect` or `ConnectToNode` while an active stop is still in `ProcessState::Stopping`
- **THEN** the new connection attempt SHALL NOT spawn until the stopping session finishes reporting `Stopped`
