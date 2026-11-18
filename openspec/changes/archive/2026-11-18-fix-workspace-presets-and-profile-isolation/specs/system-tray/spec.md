# Spec Delta

## ADDED Requirements

### Requirement: System tray item identity is scoped to the active profile
The StatusNotifierItem identifier published to the system tray SHALL incorporate the active runtime profile identifier so concurrent profile instances register distinct tray items on the session bus.

#### Scenario: Production and custom profile publish distinct tray IDs
- **WHEN** a `production` instance and a `custom:work` instance both spawn their system tray service
- **THEN** each tray item SHALL advertise a distinct profile-scoped `id` rather than colliding on `"v2ray-rs"`
