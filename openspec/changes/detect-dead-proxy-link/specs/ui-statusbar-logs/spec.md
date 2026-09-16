## ADDED Requirements

### Requirement: Status bar shows connection health
While the connection is `Running`, the status bar SHALL reflect connection health. An unhealthy session SHALL show `Proxy not responding` as the status text, with the connection details followed by the last probe failure. A healthy session whose DNS through the proxy is marked failing SHALL show `Connected` with the details prefixed by `DNS via proxy failing`. A healthy session without the DNS mark SHALL show the existing connected text. The connect button SHALL keep its connected appearance in all three cases.

#### Scenario: Unhealthy status text
- **WHEN** the connection is `Running` and unhealthy with last failure `certificate has expired`
- **THEN** the status text SHALL be `Proxy not responding` and the details SHALL end with the failure reason

#### Scenario: DNS failing status text
- **WHEN** the connection is `Running`, healthy, and DNS through the proxy is marked failing
- **THEN** the status text SHALL be `Connected` and the details SHALL start with `DNS via proxy failing`

#### Scenario: Health recovers
- **WHEN** an unhealthy session becomes healthy with no DNS mark
- **THEN** the status bar SHALL show the same text as any connected session
