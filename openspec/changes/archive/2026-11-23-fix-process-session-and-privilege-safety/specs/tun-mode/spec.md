# Spec Delta: tun-mode

## ADDED Requirements

### Requirement: Startup preflight checks nosuid on canonical backend and helper paths
Startup preflight SHALL canonicalize the backend binary path and check `file_caps_supported` on the canonical backend, route helper, and anti-DPI helper paths, while `mount_for_path` skips blank or short mount lines and `getfacl` probes cannot deadlock on stderr.

#### Scenario: Symlinked backend pointing onto a nosuid mount is caught at preflight
- **WHEN** `binary_path` is a symlink whose target lies on a `nosuid` mount
- **THEN** startup preflight SHALL reject the start with `nosuid` guidance before spawning

#### Scenario: Helper on a nosuid mount fails preflight cleanly
- **WHEN** `route_helper` or `antidpi.helper_path` resolves to a `nosuid` mount
- **THEN** startup preflight SHALL refuse before spawn instead of failing later with `EPERM`

#### Scenario: Blank line in mount table does not abort parsing
- **WHEN** `/proc/self/mounts` text contains a blank or malformed line before the matching mount
- **THEN** `mount_for_path` SHALL skip it and still match the target mount entry

### Requirement: Bypass wrapper tolerates non-UTF-8 inherited environment variables
`v2ray-rs-run` SHALL inspect inherited environment variables using `std::env::vars_os` so non-UTF-8 variables outside the allowlist never cause a panic.

#### Scenario: Non-UTF-8 unrelated env var does not crash v2ray-rs-run
- **WHEN** `v2ray-rs-run` runs with an inherited non-UTF-8 environment variable that is not in the allowlist
- **THEN** `sanitize_env` SHALL drop it cleanly without panicking
