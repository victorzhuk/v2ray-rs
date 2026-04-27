## MODIFIED Requirements

### Requirement: Tray notifications
The system SHALL support optional desktop notifications for tray-visible connection state changes.

#### Scenario: Notifications enabled
- **WHEN** notifications are enabled and the connection state changes to Running or Error
- **THEN** the tray integration SHALL emit a desktop notification describing the state change

