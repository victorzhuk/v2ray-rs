# Spec: Backend Detection

## Purpose

Detect installed v2ray/xray/sing-box binaries, query their versions, validate custom paths, persist the user's backend selection, and expose backend-specific Real Delay capability.

## Requirements

### Requirement: Auto-detect installed backends
The system SHALL keep detected backend binaries visible even when version probing fails, marking them unavailable instead of silently omitting the failure.

#### Scenario: Version probe failure remains visible
- **WHEN** a backend binary exists but `version` probing fails
- **THEN** the backend remains listed in onboarding/preferences, is disabled for selection, and displays the probe error

#### Scenario: Single usable backend installed
- **WHEN** exactly one detected backend is available for use
- **THEN** onboarding SHALL auto-select that backend

### Requirement: Backend version detection
The system SHALL query each detected backend's version by executing the binary with appropriate arguments and parsing stdout.

#### Scenario: Successful version query
- **WHEN** `/usr/bin/v2ray` is detected
- **THEN** the system SHALL run `v2ray version` and store the version string

#### Scenario: Binary exists but fails to run
- **WHEN** a binary exists but returns an error on version query
- **THEN** the system SHALL mark that backend as unavailable with the error message

### Requirement: Custom backend path
The system SHALL validate custom backend paths strictly before accepting them.

#### Scenario: Version probe fails for custom path
- **WHEN** the user enters an executable custom path whose `version` command fails
- **THEN** the system SHALL reject the path and show the validation error instead of saving it

### Requirement: Backend selection persistence
The system SHALL persist the user's backend selection so it survives app restarts.

#### Scenario: Restart preserves selection
- **WHEN** the user selects sing-box and restarts the app
- **THEN** sing-box SHALL remain the selected backend

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
