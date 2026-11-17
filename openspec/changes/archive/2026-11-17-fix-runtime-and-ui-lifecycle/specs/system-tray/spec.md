# Spec Delta

## MODIFIED Requirements

### Requirement: Minimize to tray
The system SHALL support minimizing the main window to the system tray instead of closing when a `StatusNotifierWatcher` host is registered on the session D-Bus; when no tray host is registered, closing the window SHALL quit the application cleanly instead of leaving an unreachable hidden window.

#### Scenario: Close button minimizes
- **WHEN** the user clicks the window close button, minimize-to-tray is enabled, and `StatusNotifierWatcher.IsStatusNotifierHostRegistered` is `true` on session D-Bus
- **THEN** the main window SHALL be hidden and the app SHALL continue running in the tray

#### Scenario: Close button quits when no tray host is registered
- **WHEN** the user clicks the window close button, minimize-to-tray is enabled, and no `StatusNotifierHost` is registered on session D-Bus (or D-Bus query fails)
- **THEN** the application SHALL run its normal quit path instead of hiding the window

#### Scenario: Restore from tray
- **WHEN** the user clicks "Open Main Window" in the tray menu
- **THEN** the main window SHALL be shown and focused

## ADDED Requirements

### Requirement: Tray listener resyncs on lag, escapes notifications, and cleans icon resources
`TrayService` SHALL resync tray state from `StateManager` when its broadcast receiver reports `Lagged`, abort its listener task on `shutdown()`, honor the `notifications` enabled flag while escaping dynamic markup in `Notifier`, and remove temporary icon directories on exit while running `gtk-update-icon-cache` asynchronously off the GTK thread.

#### Scenario: Broadcast lag resyncs tray icon to current state
- **WHEN** rapid log events cause `RecvError::Lagged` on the tray broadcast receiver across a state transition to `Running`
- **THEN** the tray listener SHALL read the current state snapshot and update the tray icon and menu to `Connected`
