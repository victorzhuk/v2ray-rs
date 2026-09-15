## ADDED Requirements

### Requirement: Route recovery failures are visible
When a TUN route-recovery pass fails or times out, the system SHALL notify the user with the manual recovery command for the recorded backend and interface, and SHALL log the failure. The recovery pass SHALL be bounded by the same timeout as other route-helper invocations.

#### Scenario: Recovery helper fails at startup
- **WHEN** the application starts with a TUN recovery marker and the route helper's recovery exits with a non-zero status
- **THEN** the user SHALL see a notification containing the manual recovery command naming the backend flag and the interface

#### Scenario: Recovery helper hangs
- **WHEN** the recovery helper does not finish within the route-helper timeout
- **THEN** it SHALL be killed and the user SHALL see the same notification stating that recovery timed out
