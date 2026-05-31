## ADDED Requirements

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
