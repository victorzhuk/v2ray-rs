# Spec Delta: tun-preferences-ui

## ADDED Requirements

### Requirement: Grant confirmation verifies expected post-grant paths and respects targeting
`grant_confirmed` SHALL compare post-grant probe paths against expected post-grant destination paths (relocated under `/usr/local/lib/v2ray-rs` when relocation applies) and SHALL only require `AntidpiVerdict::Ready` when the anti-DPI helper was targeted by the grant.

#### Scenario: First-time AppImage relocating grant confirms cleanly
- **WHEN** a user grants privileges from a `nosuid` AppImage mount and both helpers relocate to `/usr/local/lib/v2ray-rs/`
- **THEN** `grant_confirmed` SHALL compare against the relocated paths and return `true`

#### Scenario: Untargeted fixable anti-DPI helper does not fail a TUN-only grant
- **WHEN** `grant_confirmed` runs with `antidpi_targeted == false` and the anti-DPI probe returns `AntidpiVerdict::Fixable`
- **THEN** `grant_confirmed` SHALL return `true`

### Requirement: TUN preferences avoid widget reference cycles, main-thread probes, and untranslated verdicts
`build_tun_page` SHALL hold a `Weak` reference in its refresh closure to avoid leaking widgets, SHALL resolve helper and wrapper paths off the GTK main thread, and SHALL provide gettext catalog entries for every `tr(...)` capability verdict string.

#### Scenario: Helper path resolution runs off the GTK main thread
- **WHEN** the user opens TUN preferences or clicks Grant
- **THEN** `run_path()`, `helper_path()`, and `antidpi_path()` SHALL run inside `spawn_blocking` rather than synchronously on the GTK thread

#### Scenario: All anti-DPI capability verdicts translate in ru_RU
- **WHEN** the Russian `.mo` catalog is inspected for the four anti-DPI verdict strings from `preferences/tun.rs`
- **THEN** every `msgid` SHALL be present with a Russian translation

### Requirement: Strategy editor rejects multi-newline blank inputs
`strategies_from_text` SHALL treat only `""` and a single trailing `"\n"` as clearing the saved strategy list and SHALL reject multi-newline blank inputs such as `"\n\n"` as invalid blank lines.

#### Scenario: Double newline does not clear saved strategies
- **WHEN** the user enters `"\n\n"` in the strategy editor and focuses out
- **THEN** `strategies_from_text` SHALL return an error and keep the previously saved strategy list
