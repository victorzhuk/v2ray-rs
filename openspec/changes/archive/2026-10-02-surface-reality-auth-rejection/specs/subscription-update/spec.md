## ADDED Requirements

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
