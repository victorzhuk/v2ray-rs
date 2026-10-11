# geo-availability-probe Specification

## Purpose
TBD - created by archiving change geo-availability-probe. Update Purpose after archive.

## Requirements

### Requirement: User-triggered availability probe per node
The system SHALL provide a user-triggered availability probe that, for each selected node, requests each built-in service's check URL through that node's own tunnel and derives a per-service verdict of `available`, `blocked`, or `unknown`. The probe SHALL reuse the ephemeral probe session and per-node loopback inbounds used by Real Delay, SHALL NOT disturb the active user-facing connection, and SHALL be subject to the same single-session concurrency rule.

#### Scenario: Probe a selection of nodes
- **WHEN** the user invokes "Check availability" on one or more enabled nodes
- **THEN** the system SHALL produce, for each selected node and each built-in service, a verdict of `available`, `blocked`, or `unknown`, and SHALL surface the results without stopping or reconfiguring the user-facing backend

#### Scenario: Probe request follows the gate redirect
- **WHEN** a service check through a node answers with a redirect chain that terminates within the redirect cap
- **THEN** the verdict derivation SHALL evaluate the final URL and body of the chain, not the first hop

#### Scenario: Transport failure is unknown, not blocked
- **WHEN** the service request through a node times out or fails to connect
- **THEN** the verdict SHALL be `unknown` and SHALL NOT be recorded as `blocked`

#### Scenario: Concurrency with Real Delay
- **WHEN** an availability probe is requested while a Real Delay session is running
- **THEN** the request SHALL be queued or rejected with a user-visible toast, and SHALL NOT spawn a second ephemeral backend

### Requirement: Built-in service checks are data-defined
The system SHALL define service checks as a data table of `{id, display name, check URL, blocked markers}`, where blocked markers match the final URL or the response body. The built-in table SHALL initially contain NotebookLM (`https://notebooklm.google.com/`, blocked on a `location=unsupported` final-URL marker or an availability-refusal body marker) and Gemini (`https://gemini.google.com/about`, blocked on an availability-refusal body marker). Adding or editing a service SHALL NOT require protocol or backend changes.

#### Scenario: NotebookLM rejected exit
- **WHEN** the NotebookLM check through a node terminates at a URL containing `location=unsupported`
- **THEN** the verdict for NotebookLM on that node SHALL be `blocked`

#### Scenario: NotebookLM accepted exit
- **WHEN** the NotebookLM check through a node terminates on the sign-in flow or a landing page without a blocked marker
- **THEN** the verdict for NotebookLM on that node SHALL be `available`

#### Scenario: Gemini refusal marker
- **WHEN** the Gemini check through a node returns a 200 body containing the availability-refusal marker
- **THEN** the verdict for Gemini on that node SHALL be `blocked`

### Requirement: Availability verdict persistence
The system SHALL persist the most recent verdict per node per service, with its checked-at timestamp, in the latency snapshot file. A stale or missing verdict SHALL be representable as unknown and SHALL NOT be displayed as a fresh result.

#### Scenario: Verdict written after a probe
- **WHEN** an availability probe derives a verdict for a node and service
- **THEN** the verdict and checked-at timestamp SHALL be persisted atomically to the latency snapshot file

#### Scenario: Startup hydration of verdicts
- **WHEN** the app launches and the latency snapshot contains verdicts
- **THEN** the availability view SHALL show the persisted verdicts with their original checked-at times without running a probe

### Requirement: Availability results surface in the UI
The system SHALL present availability verdicts in a view listing the probed nodes against the built-in services, with a badge per verdict and its checked-at time. For a `blocked` verdict, the view SHALL show the node's last known egress sample when one exists, so the rejected exit is visible next to the verdict.

#### Scenario: Results view renders per node and service
- **WHEN** an availability probe completes for N nodes and M services
- **THEN** the view SHALL show N×M badges with each verdict and its checked-at time

#### Scenario: Blocked verdict shows the exit
- **WHEN** a node's NotebookLM verdict is `blocked` and the node has a persisted egress sample
- **THEN** the view SHALL show that egress next to the blocked badge

#### Scenario: Unknown verdict is distinct
- **WHEN** a node's service check ended as `unknown`
- **THEN** the view SHALL show an unknown badge and SHALL NOT style it as either available or blocked
