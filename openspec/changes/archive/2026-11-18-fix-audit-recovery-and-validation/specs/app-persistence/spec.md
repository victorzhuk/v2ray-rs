## MODIFIED Requirements

### Requirement: Versioned TUN ownership journal
The system SHALL write `tun_session.v1.json` atomically with owner-only access. It SHALL record version, session identity, boot and network namespace identity, controller process identity, device and reservation indices, phase, observed route/rule objects, and pending addition intent. A different application run SHALL NOT resume unfinished state without recovery.

#### Scenario: Mutation is interrupted
- **WHEN** the application or helper exits after recording intent
- **THEN** pending intent SHALL survive reload and SHALL NOT be assumed complete

#### Scenario: Controller identity changed
- **WHEN** a different application process tries to reuse unfinished recovery state
- **THEN** preparation SHALL refuse without replacing the prior journal

### Requirement: Unsafe journal reads fail closed
Journal reads SHALL be bounded and SHALL refuse symlinks, nonregular files, permissive modes, and ownership mismatches. Unsupported versions, malformed records, epoch mismatches with outstanding ownership, and failed recovery SHALL retain the original journal and refuse automatic mutation.

#### Scenario: Boot or namespace differs
- **WHEN** outstanding ownership or pending intent belongs to another boot or network namespace
- **THEN** automatic mutation SHALL be refused and the journal SHALL remain intact

#### Scenario: Fresh start after a finished session from another epoch
- **WHEN** a valid journal from another boot or namespace records Complete with no resources or pending intent
- **THEN** session preparation SHALL replace the journal with a fresh session identity without changing kernel state

#### Scenario: Fresh start after an unstarted sing-box session
- **WHEN** a valid sing-box Prepared journal has no device index, reservation index, resources, or pending intent
- **THEN** session preparation SHALL replace that journal without kernel cleanup, even after a boot or namespace change

#### Scenario: Journal cannot be safely interpreted
- **WHEN** a journal is malformed or from an unsupported version
- **THEN** the system SHALL report pending recovery without deleting or overwriting it

### Requirement: Legacy recovery evidence survives migration
The legacy `tun_session.json` SHALL remain diagnostic evidence and SHALL NOT be silently converted into kernel ownership. A pending legacy record SHALL block new sessions. Rollback to an older release SHALL require verified network cleanup first.

#### Scenario: Legacy marker remains intact
- **WHEN** a legacy marker is present
- **THEN** preparation SHALL refuse and the marker SHALL remain byte-identical until explicit recovery

### Requirement: Preserve generated configuration on routing load failure
When saved routing rules cannot be loaded, regeneration SHALL fail with a reported error and SHALL leave the existing generated config file byte-identical. The system SHALL NOT write the file and SHALL NOT substitute a default-empty rule set.

#### Scenario: Routing rules fail to load
- **WHEN** regeneration runs while the saved routing rules cannot be loaded
- **THEN** regeneration SHALL report the error and the existing generated config file SHALL remain byte-identical
