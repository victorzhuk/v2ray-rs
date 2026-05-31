# Spec: Real Delay Capability Reset

## Purpose

Define how Real Delay availability, probe lifetime, and preferences behave when backend capability differs by selected backend or changes during a probe.

## Requirements

### Requirement: Real Delay capability reflects backend-specific probe surface

The system SHALL expose Real Delay availability per selected backend according to the backend-specific delay-test surface available to v2ray-rs: sing-box requires Clash API support, xray requires queryable ObservatoryService support, and v2fly/v2ray-core requires queryable ObservatoryService support.

#### Scenario: xray and v2ray start as potentially supported
- **WHEN** the selected backend is xray or v2ray and no observatory capability probe has completed for the current binary path
- **THEN** the Subscriptions UI MAY enable the Real Delay action
- **AND** the UI SHALL explain that ObservatoryService availability is checked when the probe runs

#### Scenario: xray with observatory support enables Real Delay
- **WHEN** the selected backend is xray and the app can start a probe config exposing `ObservatoryService`
- **THEN** the Subscriptions UI SHALL enable the Real Delay action for xray

#### Scenario: v2ray with observatory support enables Real Delay
- **WHEN** the selected backend is v2ray and the app can start a probe config exposing `ObservatoryService`
- **THEN** the Subscriptions UI SHALL enable the Real Delay action for v2ray

#### Scenario: backend without required probe surface disables Real Delay
- **WHEN** the selected backend cannot expose the API surface required by its Real Delay implementation
- **THEN** the Subscriptions UI SHALL disable the Real Delay action
- **AND** the disabled action SHALL explain which backend feature is required

#### Scenario: backend path change resets Real Delay capability
- **WHEN** the user changes the selected backend type or binary path
- **THEN** any in-session Real Delay capability result for the previous backend SHALL be discarded
- **AND** the new backend SHALL return to its default capability state

### Requirement: Real Delay probe has bounded wall-clock lifetime

The system SHALL enforce a hard outer timeout on the entire Real Delay async command, including process spawn, observatory polling, and backend shutdown. The timeout SHALL be strictly greater than the configured probe timeout plus a fixed margin for process cleanup.

#### Scenario: hung backend does not lock UI state
- **WHEN** the ephemeral probe backend hangs during startup, polling, or shutdown
- **THEN** the system SHALL still clear the `testing_real_delay` state for the affected subscription within the hard outer timeout
- **AND** the system SHALL report the probe as failed with an appropriate diagnostic

#### Scenario: stale result discarded after backend switch
- **WHEN** a Real Delay probe is in-flight and the user changes the backend type or binary path
- **THEN** the system SHALL discard any result from the stale probe when it arrives
- **AND** the Real Delay capability SHALL be reset to the default for the new backend

### Requirement: Real Delay preference controls persist

The system SHALL wire all Real Delay preference controls (enabled toggle, test URL, timeout, use-for-lowest-latency toggle) so that user changes are immediately persisted to `AppSettings.real_delay`.

#### Scenario: toggling Real Delay enabled persists
- **WHEN** the user toggles the Real Delay enabled switch in Preferences
- **THEN** the setting SHALL be persisted and the subscriptions page SHALL reflect the new state
