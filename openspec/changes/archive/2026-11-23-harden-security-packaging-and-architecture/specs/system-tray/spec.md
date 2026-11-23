# Spec Delta: system-tray

## MODIFIED Requirements

### Requirement: Tray listener resyncs on lag, escapes notifications, and cleans icon resources
`TrayService` SHALL resync tray state from `StateManager` when its broadcast receiver reports `Lagged`, abort its listener task on `shutdown()`, honor the `notifications` enabled flag while escaping dynamic markup in `Notifier`, and avoid creating temporary icon theme directories or spawning `gtk-update-icon-cache` during normal pixmap-only startup.

#### Scenario: Broadcast lag resyncs tray icon to current state
- **WHEN** rapid log events cause `RecvError::Lagged` on the tray broadcast receiver across a state transition to `Running`
- **THEN** the tray listener SHALL read the current state snapshot and update the tray icon and menu to `Connected`

#### Scenario: Pixmap-only startup creates no temp icon theme
- **WHEN** `TrayService::spawn` starts with `icon_name()` returning an empty string
- **THEN** it SHALL render icons via `icon_pixmap()` without creating a temp hicolor directory or invoking `gtk-update-icon-cache`
