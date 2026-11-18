# Spec Delta

## ADDED Requirements

### Requirement: Concurrent latency snapshot mutations serialize atomically
All read-modify-write updates to the persisted latency, Real Delay, egress, and availability snapshot SHALL execute under a single store-level serialization lock so concurrent probe completions, sample invalidations, and connection-time prunes never overwrite sibling updates.

#### Scenario: Concurrent subscription latency completions preserve both results
- **WHEN** two enabled subscriptions finish latency or Real Delay probes concurrently and persist their updated samples
- **THEN** the resulting `latency_snapshot.json` and in-memory snapshot SHALL retain the recorded samples from both subscriptions

#### Scenario: Connect-time prune does not drop an in-flight probe write
- **WHEN** a connection attempt prunes stale node entries from the latency snapshot while a subscription probe update is saving
- **THEN** both mutations SHALL serialize in lock order without lost updates

### Requirement: Custom routing preset filenames avoid slug collisions
Saving and deleting custom routing presets SHALL preserve distinct presets whose names normalize to the same ASCII slug by disambiguating colliding filenames with a deterministic hash suffix and verifying the stored preset name before overwriting or deleting a file.

#### Scenario: Distinct punctuation variants do not overwrite each other
- **WHEN** the user saves a custom preset named `"My Preset"` and then saves another custom preset named `"my-preset"`
- **THEN** both presets SHALL exist on disk and appear when custom presets are loaded, and saving `"My Preset"` again SHALL update only `"My Preset"`

#### Scenario: Deleting one colliding preset leaves the sibling intact
- **WHEN** both `"My Preset"` and `"my-preset"` are saved and the user deletes `"my-preset"`
- **THEN** `"my-preset"` SHALL be removed and `"My Preset"` SHALL remain intact on disk
