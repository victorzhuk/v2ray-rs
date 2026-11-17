# Spec Delta

## ADDED Requirements

### Requirement: Version probes drain pipes concurrently and skip redundant onboarding scans
Backend version and capability probes SHALL drain child `stdout` and `stderr` concurrently with bounded byte caps while polling process exit so verbose output never deadlocks on a full OS pipe buffer. When `onboarding_complete` is already `true` at startup, the onboarding wizard SHALL NOT run an initial `detect_all()` scan.

#### Scenario: Verbose backend output completes within probe timeout
- **WHEN** a probed backend writes more than 64 KiB across `stdout` and `stderr` before exiting
- **THEN** the probe SHALL drain both pipes without deadlocking and return bounded output

#### Scenario: Completed onboarding skips startup wizard backend scan
- **WHEN** the application starts with `onboarding_complete = true`
- **THEN** `OnboardingWizard` SHALL NOT invoke `detect_all()` during initialization
