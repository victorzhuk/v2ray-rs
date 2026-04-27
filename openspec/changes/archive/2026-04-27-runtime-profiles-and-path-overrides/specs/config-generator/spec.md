# Spec Delta: config-generator

## ADDED Requirements

### Requirement: Generated configs live in the runtime directory
The system SHALL write generated backend config files to the active profile's `runtime_dir/generated/` by default. The existing `backend.config_output_dir` user setting SHALL continue to override the output directory when set.

#### Scenario: Default output path
- **WHEN** the user has not set `backend.config_output_dir` and the active profile is `Production`
- **THEN** the generated `xray.json`/`v2ray.json`/`sing-box.json` SHALL be written under `runtime_dir/generated/`

#### Scenario: User override still wins
- **WHEN** the user has set `backend.config_output_dir` to `/etc/v2ray-rs/configs`
- **THEN** the generated config SHALL be written under `/etc/v2ray-rs/configs/`

#### Scenario: Generated configs are profile-isolated
- **WHEN** the same user runs the binary with `--profile production` and `--profile development` at different times
- **THEN** each profile SHALL maintain its own generated config files in its own `runtime_dir/generated/`
