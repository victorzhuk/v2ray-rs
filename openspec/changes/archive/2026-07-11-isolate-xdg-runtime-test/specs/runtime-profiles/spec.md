## ADDED Requirements

### Requirement: XDG runtime and state directory fallback
The system SHALL resolve the runtime and state directories through an injectable environment source. When `XDG_RUNTIME_DIR` is present, `runtime_dir` SHALL be that directory joined with the active profile's qualifier; when it is absent, `runtime_dir` SHALL fall back to `data_dir/runtime`. When `XDG_STATE_HOME` is present, `state_dir` SHALL be that directory joined with the qualifier; when it is absent, `state_dir` SHALL fall back to `data_dir/state`. Because resolution reads through the injected source rather than the process environment directly, the fallback SHALL be verifiable without mutating process-global environment variables.

#### Scenario: XDG_RUNTIME_DIR present
- **WHEN** the environment source reports a value for `XDG_RUNTIME_DIR`
- **THEN** `runtime_dir` SHALL be that value joined with the active profile's qualifier

#### Scenario: XDG_RUNTIME_DIR absent
- **WHEN** the environment source reports no value for `XDG_RUNTIME_DIR`
- **THEN** `runtime_dir` SHALL be `data_dir/runtime`

#### Scenario: XDG_STATE_HOME present
- **WHEN** the environment source reports a value for `XDG_STATE_HOME`
- **THEN** `state_dir` SHALL be that value joined with the active profile's qualifier

#### Scenario: XDG_STATE_HOME absent
- **WHEN** the environment source reports no value for `XDG_STATE_HOME`
- **THEN** `state_dir` SHALL be `data_dir/state`

#### Scenario: Fallback verified without touching process environment
- **WHEN** a test supplies an environment source that omits `XDG_RUNTIME_DIR` and `XDG_STATE_HOME`
- **THEN** the resolver SHALL return the `data_dir` fallbacks without reading or mutating the real process environment
