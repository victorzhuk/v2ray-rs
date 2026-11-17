# Spec Delta

## ADDED Requirements

### Requirement: Non-ASCII preset slugs, distinct non-zero ports, and serialized flushes
Custom preset filenames SHALL append a deterministic hash suffix when a preset name contains no ASCII alphanumeric characters, `AppSettings` validation SHALL reject port `0` or `socks_port == http_port`, pending debounced settings saves SHALL flush immediately on Preferences close and application quit, background subscription saves SHALL serialize with reloads, and subscription payload decoding/parsing SHALL run on `spawn_blocking`.

#### Scenario: Distinct Cyrillic preset names produce distinct files
- **WHEN** the user saves two custom routing presets named `"Работа"` and `"Дом"`
- **THEN** both presets SHALL persist to distinct `.json` filenames without overwriting each other or producing `".json"`

#### Scenario: Closing Preferences or quitting flushes pending debounced edits
- **WHEN** the user edits a setting and closes Preferences or quits within the 300 ms debounce window
- **THEN** the pending `AppSettings` state SHALL be written to `settings.toml` before close or exit completes
