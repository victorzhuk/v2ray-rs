# Spec: onboarding-wizard

## ADDED Requirements

### Requirement: First-time onboarding wizard
The system SHALL display a setup wizard on first launch to guide the user through initial configuration.

#### Scenario: First launch
- **WHEN** the app launches for the first time (no existing config)
- **THEN** the system SHALL display the onboarding wizard instead of the main window

#### Scenario: Subsequent launches
- **WHEN** the app launches with existing configuration
- **THEN** the system SHALL skip the wizard and show the main window directly

### Requirement: Wizard steps
The wizard SHALL guide the user through: welcome, backend detection, subscription import, and completion.

#### Scenario: Backend detection step
- **WHEN** the wizard reaches the backend step
- **THEN** it SHALL show detected backends, let the user select one, or show installation guidance if none found

#### Scenario: Subscription import step
- **WHEN** the wizard reaches the subscription step
- **THEN** it SHALL allow entering a subscription URL or skipping this step

#### Scenario: Completion
- **WHEN** the wizard is completed
- **THEN** the system SHALL save settings, mark onboarding as done, and transition to the main window
