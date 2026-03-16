## ADDED Requirements

### Requirement: Custom nodes persistence
The system SHALL persist manual proxy nodes in `custom_nodes.json` under the app data directory using atomic writes.

#### Scenario: Save and reload manual nodes
- **WHEN** manual nodes are saved and the app restarts
- **THEN** the nodes round-trip with their IDs, enabled state, and protocol data intact

#### Scenario: Corrupt custom nodes file
- **WHEN** `custom_nodes.json` is unreadable or invalid
- **THEN** the system falls back to an empty manual-node list and reports the problem without preventing startup

### Requirement: Restore launched manual nodes on discard
The system SHALL restore the launched manual-node set when the user discards connected-state manual-node changes.

#### Scenario: Discard connected manual-node edits
- **WHEN** the backend is connected, the user changes manual nodes, and then selects `Discard`
- **THEN** the system overwrites the current persisted manual-node set with the last launched manual-node snapshot and clears the pending divergence state
