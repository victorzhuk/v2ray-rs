## MODIFIED Requirements

### Requirement: Build ordered connection candidates
The system SHALL build an ordered list of connection candidates from enabled subscription nodes and enabled manual nodes according to the selected strategy. The Lowest Latency strategy SHALL be configurable via the `use_real_delay_for_lowest_latency` app setting: when the setting is true and a node has a recorded `last_real_delay_ms`, the system SHALL rank by Real Delay; otherwise it SHALL fall back to `last_latency_ms` (TCP). Nodes with neither sample SHALL be placed last.

#### Scenario: List order
- **WHEN** the strategy is set to list order
- **THEN** candidates SHALL follow subscription order, then node order within each subscription

#### Scenario: Lowest latency by TCP (default)
- **WHEN** the strategy is set to lowest latency and `use_real_delay_for_lowest_latency` is false
- **THEN** candidates SHALL be ordered by ascending `last_latency_ms` with unknown values placed last

#### Scenario: Lowest latency by Real Delay
- **WHEN** the strategy is set to lowest latency and `use_real_delay_for_lowest_latency` is true
- **THEN** candidates SHALL be ordered by ascending `last_real_delay_ms` for nodes that have a sample, then by ascending `last_latency_ms` for nodes that lack a Real Delay sample, then by remaining order with unknown values placed last

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
