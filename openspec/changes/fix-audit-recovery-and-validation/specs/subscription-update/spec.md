## ADDED Requirements
### Requirement: Safe subscription imports
The system SHALL exclude arbitrary HTTP error bodies from routine errors and SHALL bound regular local-file imports to 10 MiB without blocking asynchronous workers.
#### Scenario: Oversized local source
- **WHEN** a local source exceeds 10 MiB or is not a regular file
- **THEN** the import SHALL fail without replacing existing subscription data
### Requirement: Connection identity reconciliation
The system SHALL match unchanged connection settings before endpoint fallbacks, ignoring remarks. Endpoint fallback SHALL apply only to unique unmatched pairs. Changed connection settings SHALL invalidate latency samples.
#### Scenario: Reordered endpoint duplicates
- **WHEN** distinct connection configurations sharing an endpoint are reordered
- **THEN** each configuration SHALL retain its own identity and enabled state
