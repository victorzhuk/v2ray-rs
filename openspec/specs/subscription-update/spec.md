## Purpose

Defines how subscriptions are fetched, retried, and reconciled — including fetch-error classification (transient vs terminal), automatic periodic updates, and node reconciliation that preserves user preferences.

## Requirements

### Requirement: Manual subscription update
The system SHALL surface partial parse failures without discarding valid nodes.

#### Scenario: Manual update with mixed valid and invalid URIs
- **WHEN** an update source returns valid proxy URIs together with invalid entries
- **THEN** the system SHALL keep the valid nodes, report the skipped entries, and persist the reconciled result

### Requirement: Automatic subscription update
The system SHALL support automatic periodic updates of subscriptions based on a configurable interval.

#### Scenario: Auto-update triggers
- **WHEN** the configured auto-update interval elapses
- **THEN** the system SHALL fetch and update all subscriptions with auto-update enabled

#### Scenario: Auto-update failure
- **WHEN** an auto-update fails due to a transient error (HTTP 408, 429, any 5xx, connection failure, or timeout)
- **THEN** the system SHALL retry up to 3 times with exponential backoff and log the failure

#### Scenario: Terminal fetch error fails fast
- **WHEN** a subscription fetch fails with a terminal error (any other 4xx status, a malformed or non-http(s) URL, or a request that cannot be constructed)
- **THEN** the system SHALL fail immediately without retrying or sleeping and SHALL report an error that identifies the terminal cause

### Requirement: Update node reconciliation
The system SHALL reconcile updated subscription nodes with existing data, matching nodes by address, port, and protocol to preserve user preferences.

#### Scenario: Matching node keeps enabled state
- **WHEN** an updated subscription still contains a previously known node
- **THEN** the system SHALL preserve that node's enabled state

### Requirement: Fetch error classification
The subscription fetcher SHALL distinguish malformed or unsupported URLs from network-level failures in its error type, so callers and error messages do not present a permanently invalid source as a transient network problem.

#### Scenario: Malformed URL reported distinctly
- **WHEN** a subscription source URL is not a valid http(s) URL
- **THEN** the fetch error SHALL identify it as an invalid URL rather than a network error

### Requirement: Subscription refresh is offered on REALITY auth rejection
When a connect candidate's failure is classified as REALITY auth rejection and the candidate's node belongs to a subscription, the failure report SHALL offer a refresh action for that subscription. The offer SHALL appear at most once per connection per subscription regardless of how many of the subscription's nodes were classified, and the refresh itself SHALL remain user-initiated through the existing manual update flow.

#### Scenario: Offer names the owning subscription
- **WHEN** a REALITY auth rejection is classified on a node from subscription S
- **THEN** the failure report SHALL offer a refresh action for S

#### Scenario: One offer per connection per subscription
- **WHEN** candidates from subscription S are classified twice within one connection
- **THEN** the refresh offer SHALL appear exactly once

#### Scenario: Refresh stays user-initiated
- **WHEN** the classification fires
- **THEN** the system SHALL NOT start a subscription update on its own; the update SHALL run only after the user activates the offered action

#### Scenario: Classification on a manual node offers nothing
- **WHEN** a REALITY auth rejection is classified on a manual node with no owning subscription
- **THEN** the failure report SHALL show the classification without a refresh offer
