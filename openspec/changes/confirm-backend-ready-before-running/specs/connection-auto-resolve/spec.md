## MODIFIED Requirements

### Requirement: Last-success metadata is owned by connection outcomes
The last-success record SHALL change only when a connection succeeds, meaning the connection is reported `Running` after its backend became ready. A connection that reaches only `Starting`, or whose backend exits or times out before it is ready, SHALL NOT change it. Saving settings from the preferences dialog SHALL NOT modify it.

#### Scenario: Preferences opened before a connect
- **WHEN** the preferences dialog is open, a connection to node B succeeds, and the user then changes any preference
- **THEN** the persisted last-success record SHALL still name node B

#### Scenario: Backend dies before ready
- **WHEN** the last-success record names node A and a connection to node B starts but B's backend exits before it is ready
- **THEN** the persisted last-success record SHALL still name node A

#### Scenario: Respawn transitions do not rewrite it
- **WHEN** a running session on node B crashes and is respawned
- **THEN** the last-success record SHALL change only when the respawned backend is reported `Running`, and SHALL NOT change on the intermediate `Starting`
