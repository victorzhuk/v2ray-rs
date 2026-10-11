## ADDED Requirements

### Requirement: Connect-candidate failures are classified from backend signatures
When a connect candidate fails and its captured backend output contains a known failure signature, the failure SHALL be reported to the user with the signature's classified cause instead of the generic failure text. The signature set SHALL include, for xray, the REALITY `received real certificate (potential MITM or redirection)` line, classified as "server rejected REALITY authentication; the subscription's REALITY keys may be stale". Failures whose output matches no signature SHALL keep the existing generic report. Each candidate SHALL be classified at most once.

#### Scenario: REALITY auth rejection is classified
- **WHEN** a connect candidate fails and its captured backend output contains the REALITY `received real certificate` line
- **THEN** the failure report SHALL name the rejected REALITY authentication cause and the stale-keys advice instead of the generic failure text

#### Scenario: Classified once per candidate
- **WHEN** the signature appears three times in one candidate's captured output before the candidate fails
- **THEN** the failure report SHALL contain the classification exactly once

#### Scenario: Unmatched failure keeps the generic report
- **WHEN** a connect candidate fails with no captured line matching any signature
- **THEN** the failure report SHALL be the existing generic failure text and SHALL NOT mention REALITY

### Requirement: Classified failures carry no retry or order changes
Classification SHALL be a reporting-only change: the candidate order, retry behavior, failover flow, and stop conditions SHALL be identical for classified and unclassified failures.

#### Scenario: Failover is unaffected by classification
- **WHEN** a classified candidate fails and further candidates remain
- **THEN** the connection SHALL proceed to the next candidate exactly as it does for unclassified failures
