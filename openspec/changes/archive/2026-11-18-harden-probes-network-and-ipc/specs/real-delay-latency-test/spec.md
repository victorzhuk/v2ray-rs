# Spec Delta

## MODIFIED Requirements

### Requirement: Probe test URL and timeout are configurable
The system SHALL accept a user-configured test URL and per-probe timeout. The system SHALL apply sane defaults: test URL `https://www.gstatic.com/generate_204`, timeout 5000 ms. The system SHALL validate the URL as a syntactically valid `http://` or `https://` URL with a non-empty host and no userinfo credentials or `#fragment` component before accepting it.

#### Scenario: Default Real Delay settings
- **WHEN** the user has never modified Real Delay settings
- **THEN** the test URL SHALL be `https://www.gstatic.com/generate_204` and the timeout SHALL be `5000` ms

#### Scenario: User changes test URL
- **WHEN** the user enters `https://cp.cloudflare.com/generate_204` and saves settings
- **THEN** subsequent Real Delay sessions SHALL use that URL and the value SHALL persist across app restarts

#### Scenario: Invalid URL rejected
- **WHEN** the user enters `not-a-url`, `ftp://example.com/`, `https://user:secret@example.com/`, or `https://example.com/generate_204#section`
- **THEN** the system SHALL reject the input with an inline validation error and SHALL NOT persist the change

## ADDED Requirements

### Requirement: Sing-box probe controller requires per-session bearer authentication
Each ephemeral `sing-box` probe configuration SHALL configure a fresh cryptographic random secret on its loopback Clash-compatible API controller, and all delay queries in that probe batch SHALL authenticate with `Authorization: Bearer <secret>`.

#### Scenario: Unauthenticated local request to probe controller is rejected
- **WHEN** an ephemeral `sing-box` probe batch is running and a local caller requests `/proxies` or `/configs` on the loopback controller port without the batch bearer token
- **THEN** the `sing-box` controller SHALL reject the unauthenticated request while the probe runner's authenticated delay queries succeed

### Requirement: Ephemeral probe child lifecycle avoids thread-bound death signals
Ephemeral probe child processes SHALL NOT configure Linux thread-scoped parent-death signals from worker threads of a multi-threaded async runtime. Termination on completion, cancellation, or drop SHALL rely on explicit graceful stop (`SIGTERM` then `SIGKILL`) and child drop-kill semantics.

#### Scenario: Worker thread retirement does not kill active probe backend
- **WHEN** an ephemeral probe backend is spawned from a multi-threaded async worker thread that exits while the probe batch is still running
- **THEN** the probe backend SHALL remain alive until the probe batch completes or is cancelled
