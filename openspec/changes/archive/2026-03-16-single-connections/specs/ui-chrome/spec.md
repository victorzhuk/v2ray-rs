## ADDED Requirements

### Requirement: Restart-required banner for manual nodes
The main window SHALL reuse the restart-required banner for connected manual-node changes that diverge from the launched runtime snapshot.

#### Scenario: Connected manual-node change
- **WHEN** the backend is connected and the user adds, edits, deletes, or toggles the enabled state of a manual node
- **THEN** a banner appears with `Apply & Restart` and `Discard` actions

#### Scenario: Discard connected manual-node change
- **WHEN** the user selects `Discard` after connected manual-node changes
- **THEN** the banner is dismissed and the persisted manual-node set returns to the launched snapshot
