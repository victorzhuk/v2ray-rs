## ADDED Requirements

### Requirement: Restart-required banner
The main window SHALL display an `adw::Banner` when the persisted runtime configuration diverges from the active runtime snapshot while the backend is starting or running.

#### Scenario: Connected DNS or routing change
- **WHEN** the backend is connected and the user changes a runtime-relevant DNS or routing setting
- **THEN** a banner appears with "Apply & Restart" and "Discard" actions

#### Scenario: Divergence resolved
- **WHEN** the user applies the restart, discards the changes, or disconnects
- **THEN** the banner is dismissed
