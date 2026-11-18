# Spec Delta

## MODIFIED Requirements

### Requirement: Non-ASCII preset slugs, distinct non-zero ports, and serialized flushes
Custom preset filenames SHALL append a fixed cross-release deterministic 64-bit hash suffix (computed over the UTF-8 preset name without `DefaultHasher`) when a preset name contains no ASCII alphanumeric characters, `AppSettings` validation SHALL reject port `0` or `socks_port == http_port`, pending debounced settings saves SHALL flush immediately on Preferences close and application quit, background subscription saves SHALL serialize with reloads, and subscription payload decoding/parsing SHALL run on `spawn_blocking`.

#### Scenario: Distinct Cyrillic preset names produce distinct files
- **WHEN** the user saves two custom routing presets named `"Работа"` and `"Дом"`
- **THEN** both presets SHALL persist to fixed, distinct `preset-<16hex>.json` filenames derived from a deterministic non-`DefaultHasher` digest without overwriting each other or producing `".json"`, and saving the same name again SHALL overwrite its existing file across Rust toolchain upgrades

#### Scenario: Closing Preferences or quitting flushes pending debounced edits
- **WHEN** the user edits a setting and closes Preferences or quits within the 300 ms debounce window
- **THEN** the pending `AppSettings` state SHALL be written to `settings.toml` before close or exit completes
