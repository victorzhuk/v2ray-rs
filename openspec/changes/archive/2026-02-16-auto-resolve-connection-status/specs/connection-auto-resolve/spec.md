## ADDED Requirements

### Requirement: Global auto-resolve strategy setting
The system SHALL allow users to select a global auto-resolve strategy that controls how enabled nodes are ordered for connection attempts.

#### Scenario: Default strategy
- **WHEN** the user has not changed the auto-resolve strategy
- **THEN** the system SHALL default to list order selection

#### Scenario: Change strategy
- **WHEN** the user selects a different auto-resolve strategy
- **THEN** the system SHALL persist the selection and use it for subsequent connections

### Requirement: Build ordered connection candidates
The system SHALL build an ordered list of connection candidates from enabled subscription nodes according to the selected strategy.

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

#### Scenario: Geo-aware
- **WHEN** the strategy is set to geo-aware
- **THEN** candidates SHALL be ordered by geo preference rules with unspecified geo data falling back to list order

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
The system SHALL track and expose metadata for the active connection, including subscription, node, latency, strategy, backend, and connected since timestamp.

#### Scenario: Successful connection metadata
- **WHEN** a connection succeeds
- **THEN** the system SHALL store the active candidate metadata and publish it to the UI and tray
