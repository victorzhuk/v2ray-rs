# Spec: Connection Auto-Resolve

**Purpose:** TBD

## ADDED Requirements

### Requirement: Global auto-resolve strategy setting
The current supported strategies SHALL be list order, lowest latency, random, and last successful.

#### Scenario: Legacy geo-aware setting
- **WHEN** persisted settings still contain `geo-aware`
- **THEN** the app SHALL migrate that value to `last-successful` on load

### Requirement: Build ordered connection candidates
The system SHALL build an ordered list of connection candidates from enabled subscription nodes and enabled manual nodes according to the selected strategy.

#### Scenario: List order
- **WHEN** the strategy is set to list order
- **THEN** candidates SHALL follow subscription order, then node order within each subscription

#### Scenario: Lowest latency
- **WHEN** the strategy is set to lowest latency
- **THEN** candidates SHALL be ordered by ascending latency with unknown latency candidates placed last

#### Scenario: Random
- **WHEN** the strategy is set to random
- **THEN** candidates SHALL be shuffled for each connection attempt

#### Scenario: Last successful
- **WHEN** the strategy is set to last successful
- **THEN** the last successful node (if available and enabled) SHALL be first, followed by remaining candidates in list order

#### Scenario: Manual node included in candidate list
- **WHEN** a manual node is enabled
- **THEN** it appears in the candidate list alongside enabled subscription nodes

#### Scenario: Last successful manual node remains stable
- **WHEN** the last successful candidate was a manual node and another manual node is inserted or deleted
- **THEN** the stored last-success reference still points to the same manual node by ID

#### Scenario: Last successful subscription node remains stable
- **WHEN** the last successful candidate was a subscription node and subscriptions are reordered or refreshed
- **THEN** the stored last-success reference still points to the same subscription node by stable node ID

### Requirement: Sequential connection attempts
The system SHALL attempt to connect using candidates in order until one succeeds or all fail.

#### Scenario: First candidate succeeds
- **WHEN** the first candidate starts successfully
- **THEN** the system SHALL mark the connection as established and stop further attempts

#### Scenario: Candidate failure
- **WHEN** a candidate fails to start
- **THEN** the system SHALL try the next candidate until success or exhaustion

#### Scenario: All candidates fail
- **WHEN** all candidates fail to start
- **THEN** the system SHALL report a connection failure and remain disconnected

### Requirement: Track connection metadata
The system SHALL track and expose connection metadata for subscription and manual node sources.

#### Scenario: Successful connection metadata
- **WHEN** a connection succeeds
- **THEN** the system SHALL store the active candidate metadata and publish it to the UI and tray

#### Scenario: Manual node connection metadata
- **WHEN** a manual node connects successfully
- **THEN** the metadata reports source `Manual`, the selected node name, strategy, backend, latency, and connected-since timestamp
