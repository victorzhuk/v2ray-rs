# Spec Delta

## ADDED Requirements

### Requirement: xray route setup reconciles optional policy rules across re-runs
Each `xray-up` run on an owned TUN session SHALL reconcile optional UID-bypass (pref 8998) and port-53 DNS-capture (pref 8999) policy rules to match the current invocation flags, removing any journaled rule at those priorities whose flag is absent on the current run.

#### Scenario: Failover from pinned to unpinned candidate removes DNS capture rules
- **WHEN** candidate 1 ran `xray-up` with `--capture-dns` and failed, and candidate 2 on the same TUN session runs `xray-up` without `--capture-dns`
- **THEN** the journaled pref 8999 UDP and TCP port-53 capture rules from candidate 1 SHALL be removed and unmarked DNS queries SHALL no longer be steered into table 2023 by pref 8999

#### Scenario: Re-running xray-up without bypass UID removes prior bypass rule
- **WHEN** an owned session previously installed a pref 8998 UID-range rule and `xray-up` runs again without `--bypass-uid`
- **THEN** the prior pref 8998 rule SHALL be removed from both kernel policy state and the ownership journal

## MODIFIED Requirements

### Requirement: Parked TUN state tears down before incompatible connect and drops failed managers
Before starting a non-TUN or differently-configured TUN connection, or when a follow-up connect from terminal `Error` aborts in preflight, the system SHALL tear down any parked fail-closed TUN routing state and journal first. Once a replacement TUN candidate adopts the active TUN session and reaches `Running`, parked failed candidate `ProcessManager`s SHALL drop immediately without running tunnel teardown or stop checks against the shared interface and journal.

#### Scenario: Switching from strict TUN Error to non-TUN Connect clears routes first
- **WHEN** strict `xray` TUN is in `Error` with fail-closed routes parked, the user disables TUN or switches to `v2ray` (including via Apply & Restart), and initiates Connect
- **THEN** the system SHALL tear down the parked `xray` TUN routes and journal before starting the non-TUN connection

#### Scenario: Preflight abort on pending reconnect from TUN Error releases routes
- **WHEN** strict `xray` TUN enters terminal `Error` with a pending reconnect, deferred connect, or direct-node connect queued, and that follow-up connect aborts in preflight (no enabled nodes, disabled target node, missing backend binary, disabled host IPv6 with an IPv6 tunnel address, routing load error, or missing geodata)
- **THEN** the system SHALL release the parked TUN routing state and clear the completed recovery journal instead of leaving fail-closed routes installed while disconnected

#### Scenario: Compatible TUN candidate failover preserves the replacement tunnel
- **WHEN** candidate 1 fails before or after TUN device creation under `xray` or `sing-box` TUN and candidate 2 on the same backend and interface reaches `Running`
- **THEN** dropping candidate 1's parked state SHALL NOT invoke `xray-down`, delete candidate 2's live TUN device or policy rules, complete candidate 2's active journal, or run sing-box absence checks against candidate 2's live interface

#### Scenario: Cleanup fails before an incompatible connect
- **WHEN** teardown of the parked session fails or cannot verify ownership
- **THEN** the system SHALL retain the journal, report the failure, and leave the requested backend unstarted
