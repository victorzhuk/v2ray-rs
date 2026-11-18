# Spec Delta

## ADDED Requirements

### Requirement: Setuid wrapper closes inherited file descriptors before execution
Before invoking `execvp`, `v2ray-rs-run` SHALL close all inherited open file descriptors greater than `2` (`stderr`) so non-`FD_CLOEXEC` descriptors held by the caller are never inherited by the `v2ray-rs-bypass` target process.

#### Scenario: Open caller descriptor >= 3 is closed across wrapper execution
- **WHEN** a caller opens a file or socket descriptor `>= 3` without `FD_CLOEXEC` and spawns a command via `v2ray-rs-run`
- **THEN** the executed target process SHALL observe only standard descriptors `0`, `1`, and `2` open from the caller
