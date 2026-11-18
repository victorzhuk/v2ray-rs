## MODIFIED Requirements
### Requirement: Safe subscription imports
The system SHALL exclude arbitrary HTTP error bodies from routine errors and SHALL bound regular local-file imports to 10 MiB without blocking asynchronous workers.
#### Scenario: Oversized local source
- **WHEN** a local source exceeds 10 MiB or is not a regular file
- **THEN** the import SHALL fail without replacing existing subscription data
#### Scenario: Server rejects a refresh
- **WHEN** the subscription server returns an HTTP error with a response body
- **THEN** the reported fetch error SHALL omit the body and the existing subscription SHALL remain intact

### Requirement: Connection identity reconciliation
The system SHALL match unchanged connection settings before endpoint fallbacks, ignoring remarks. Endpoint fallback SHALL apply only to unique unmatched pairs. Changed connection settings SHALL invalidate latency samples.
#### Scenario: Reordered endpoint duplicates
- **WHEN** distinct connection configurations sharing an endpoint are reordered
- **THEN** each configuration SHALL retain its own identity and enabled state

#### Scenario: A changed connection has an outstanding measurement
- **WHEN** unique endpoint fallback retains a node identity after its connection settings change
- **THEN** the system SHALL invalidate saved latency samples for the old settings
- **AND** a probe started with the old settings SHALL NOT restore a stale sample when it finishes
