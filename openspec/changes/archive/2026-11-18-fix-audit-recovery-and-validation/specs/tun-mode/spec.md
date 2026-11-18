## MODIFIED Requirements
### Requirement: Failed recovery remains pending
The system SHALL retain the TUN recovery marker when cleanup fails or times out and SHALL report the failure. Cleanup SHALL propagate enumeration and mutation failures while attempting independent cleanup operations.
#### Scenario: Recovery retry
- **WHEN** cleanup fails and the application restarts
- **THEN** recovery SHALL be attempted again and the marker SHALL clear only after successful cleanup

### Requirement: Journal identity gates helper commands
Automatic xray helper operations SHALL require a versioned journal and a separately supplied matching session identity. Broad legacy cleanup SHALL require an explicit operator override, never selected automatically by the desktop.

#### Scenario: Delayed stop observes a replacement journal
- **WHEN** a stop carries a session identity different from the current journal
- **THEN** the helper SHALL refuse without mutating kernel state or the journal

### Requirement: Namespace reservation prevents session collisions
The helper SHALL serialize mutations per network namespace and reserve xray routing state before backend spawn. It SHALL refuse occupied tables, reserved rule priorities, or interface names. A successful reservation SHALL bind its kernel index and session alias to the journal. Missing or mismatched reservation evidence SHALL refuse automatic mutation.

#### Scenario: Another profile owns the routing namespace
- **WHEN** another session already owns the kernel reservation
- **THEN** setup SHALL fail without changing that session's routes, rules, device, or journal

#### Scenario: Reservation tagging was interrupted
- **WHEN** a reservation exists without the recorded session alias
- **THEN** automatic recovery SHALL refuse and retain the journal

### Requirement: Cleanup verifies exact ownership
Cleanup SHALL verify the reservation, device identity, and exact recorded route/rule objects, including multiplicity, before deleting anything. Unknown effects and foreign state SHALL retain recovery state and refuse automatic mutation. Intent SHALL be written before addition and the observed object recorded after acknowledgement.

#### Scenario: Foreign state occupies a reserved priority
- **WHEN** foreign or duplicate rule objects appear at a reserved priority
- **THEN** automatic setup or cleanup SHALL refuse before deleting any resource

#### Scenario: Device name or index is reused
- **WHEN** the live device alias or index differs from the recorded identity
- **THEN** automatic teardown SHALL refuse and retain the journal

#### Scenario: Kernel acknowledgement was not recorded
- **WHEN** a mutation intent remains pending and live state contains an unrecorded object
- **THEN** automatic recovery SHALL refuse without treating the object as owned

#### Scenario: Recovery repeats after verified completion
- **WHEN** a completed session is recovered again
- **THEN** the helper SHALL succeed without changing current networking

### Requirement: Unverified sing-box recovery requires an operator
Sing-box startup SHALL remain helper-free. Automatic destructive recovery SHALL be refused when ownership of its self-managed state cannot be established.

#### Scenario: Sing-box session ended without verified cleanup
- **WHEN** a previous sing-box session has pending recovery without ownership evidence
- **THEN** the desktop SHALL retain its journal and request operator recovery without flushing tables or deleting a live device

### Requirement: Sing-box clean stop requires verified absence
Before reporting a clean sing-box TUN stop, the system SHALL verify that the device, configured table 2022, and rules selecting that table are absent. Inspection SHALL require no privileges and SHALL NOT delete resources. Failure or remaining state SHALL retain the journal and report a cleanup error. Launch intent SHALL be recorded before spawn.

#### Scenario: Foreign or leftover sing-box state remains
- **WHEN** a device, route, or selecting rule remains during stop verification
- **THEN** the system SHALL preserve it, retain the journal, and report pending recovery

#### Scenario: Sing-box never started or already completed
- **WHEN** a valid journal proves the session is unstarted Prepared or verified Complete
- **THEN** the record MAY be retired without invoking destructive helper recovery
