# Spec Delta

## ADDED Requirements

### Requirement: Directory-overridden instances bypass session-bus uniqueness
When any per-directory path override is active, the desktop application SHALL register without session-bus application uniqueness so an isolated directory-override instance starts its own window alongside a running default instance of the same profile while still enforcing the per-directory instance lock.

#### Scenario: Overridden directories launch alongside running default instance
- **WHEN** a default `production` instance already owns `com.github.v2ray-rs` on the session bus and a second process starts with `--config-dir`, `--data-dir`, `--cache-dir`, `--runtime-dir`, and `--state-dir` pointing to an isolated directory set
- **THEN** the second process SHALL open its own application window bound to the overridden directories instead of activating the primary instance and exiting

#### Scenario: Duplicate launch on the same overridden runtime directory still locks
- **WHEN** a second process starts with the same `--runtime-dir` already locked by a running overridden instance
- **THEN** startup SHALL refuse with exit code 75 naming the holding PID
