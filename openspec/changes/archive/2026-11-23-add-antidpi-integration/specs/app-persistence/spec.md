# Spec Delta: app-persistence

## ADDED Requirements

### Requirement: Anti-DPI settings persistence
The system SHALL persist anti-DPI preferences in `settings.toml` as an `[antidpi]`
section containing `enabled` (bool), `queue` (u16), `strategies` (list of strategy
chain strings), and `hostlist_path` (optional string). When the section is absent
from a previously written `settings.toml`, the system SHALL load with documented
defaults (anti-DPI disabled, queue 200, empty strategy list, no hostlist) without
prompting or erroring. An empty `strategies` list SHALL mean the helper's built-in
default strategies. A section written before this change or by a newer build SHALL
deserialize the recognized fields with defaults for missing ones and SHALL ignore
unknown fields. Settings validation SHALL reject `queue = 0`, more than 16
strategies, empty or oversized strategy strings, and a blank hostlist path.

#### Scenario: Missing antidpi section in legacy settings
- **WHEN** an existing `settings.toml` has no `[antidpi]` section
- **THEN** the system SHALL load successfully with `enabled = false`, queue `200`,
  an empty `strategies` list, and no `hostlist_path`, and SHALL NOT log an error

#### Scenario: Round-trip anti-DPI settings
- **WHEN** the user sets a strategy list and hostlist path, the settings are saved,
  and the app restarts
- **THEN** the reloaded `AppSettings.antidpi` SHALL contain the same strategies and
  hostlist path alongside the same `enabled` and `queue`

#### Scenario: Forward-compatible deserialization
- **WHEN** a `settings.toml` written by a newer build contains anti-DPI fields an
  older build does not recognize, or an `[antidpi]` section lacking keys this
  build knows
- **THEN** the build SHALL load the recognized fields, default the missing ones,
  ignore unknown ones, and SHALL NOT fail startup

#### Scenario: Invalid anti-DPI values are refused
- **WHEN** a settings file or an editor mutation carries `queue = 0`, more than 16
  strategies, an empty strategy string, or an empty `hostlist_path`
- **THEN** settings validation SHALL fail with an error naming the offending value

#### Scenario: Invalid values never reach the file
- **WHEN** a save or a legacy-migration writeback would persist an invalid
  anti-DPI configuration
- **THEN** the write SHALL be refused and the existing `settings.toml` SHALL remain
  byte-identical; the loaded settings MAY still be returned so the user can repair
  them, but they SHALL NOT be written again until they validate
