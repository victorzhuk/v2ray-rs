# Spec Delta

## MODIFIED Requirements

### Requirement: Single-instance lock per profile
The system SHALL ensure the lock parent directory exists with owner-only `0700` permissions and acquire an exclusive advisory lock on `runtime_dir/v2ray-rs.lock` without truncating existing lock/stamp files before lock acquisition, holding it for the lifetime of the process. On contention with default profile paths and an active session D-Bus primary application, a second invocation SHALL focus the primary window and exit with code 0; when per-directory overrides are active or D-Bus activation is unavailable, it SHALL report the holder PID from `instance.json` when readable -- or state that the holder PID is unknown rather than reporting the blocked caller's own PID -- without modifying `instance.json`, and exit with code 75.

#### Scenario: Second instance on default paths focuses primary window
- **WHEN** an instance is running for a profile with default paths (no `--*-dir` or `V2RAY_RS_*_DIR` overrides) and a second invocation targets the same profile in the same graphical session
- **THEN** the second invocation SHALL activate the running `com.github.v2ray-rs[.qualifier]` application window over session D-Bus, leave `instance.json` untouched, and exit with code 0

#### Scenario: Second instance of the same profile is refused
- **WHEN** an instance is already running for a given profile and a second invocation targets the same profile with `--*-dir` directory overrides or when session D-Bus activation is unavailable
- **THEN** the second invocation SHALL fail to acquire the lock, SHALL print the holder PID recorded in `instance.json` without overwriting `instance.json`, and SHALL exit with code 75

#### Scenario: Lock contention without readable stamp does not report caller PID
- **WHEN** an advisory lock on `v2ray-rs.lock` is held by another process while `instance.json` is missing or unreadable
- **THEN** lock acquisition SHALL report contention with an unknown holder PID rather than claiming the blocked process's own PID holds the lock

#### Scenario: Different profiles run side-by-side
- **WHEN** an instance is running with `--profile production` and another is started with `--profile development`
- **THEN** both instances SHALL run concurrently without contending for the same lock file

#### Scenario: Lock is released on shutdown
- **WHEN** an instance exits cleanly or is killed
- **THEN** the kernel SHALL release the advisory lock and a subsequent invocation of the same profile SHALL be able to acquire it

## ADDED Requirements

### Requirement: Subprocess log capture bounds line allocation and helper output
Backend stdout/stderr readers and route-helper output readers SHALL cap per-line byte allocation at 16 KiB, splitting or truncating newline-free output streams before allocation grows further, and route-helper runs SHALL bound total captured lines in memory.

#### Scenario: Newline-free backend output does not grow heap unboundedly
- **WHEN** a spawned backend or route helper writes a continuous 1 MiB stream of bytes with no newline character
- **THEN** the reader SHALL emit bounded segments of at most 16 KiB each without allocating a single multi-megabyte line buffer
