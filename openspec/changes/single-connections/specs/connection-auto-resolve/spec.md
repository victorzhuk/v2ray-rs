## MODIFIED Requirements

### Requirement: Build ordered connection candidates
The system SHALL build an ordered list of connection candidates from enabled subscription nodes and enabled manual nodes according to the selected strategy.

#### Scenario: Manual node included in candidate list
- **WHEN** a manual node is enabled
- **THEN** it appears in the candidate list alongside enabled subscription nodes

#### Scenario: Last successful manual node remains stable
- **WHEN** the last successful candidate was a manual node and another manual node is inserted or deleted
- **THEN** the stored last-success reference still points to the same manual node by ID

### Requirement: Track connection metadata
The system SHALL track and expose connection metadata for subscription and manual node sources.

#### Scenario: Manual node connection metadata
- **WHEN** a manual node connects successfully
- **THEN** the metadata reports source `Manual`, the selected node name, strategy, backend, latency, and connected-since timestamp
