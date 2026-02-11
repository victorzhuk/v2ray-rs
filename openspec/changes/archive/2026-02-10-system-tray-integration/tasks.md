# Tasks: System Tray Integration

## 1. Tray Icon Setup

- [x] 1.1 Add ksni and notify-rust dependencies
- [x] 1.2 Create SVG icons for three states: disconnected (gray), connected (green), error (red)
- [x] 1.3 Embed icons in binary via include_bytes!
- [x] 1.4 Implement ksni StatusNotifierItem with initial icon display

## 2. Tray Menu

- [x] 2.1 Implement context menu structure: Connect/Disconnect, separator, profile info, Open Main Window, Quit
- [x] 2.2 Implement Connect action: send start message to process manager
- [x] 2.3 Implement Disconnect action: send stop message to process manager
- [x] 2.4 Implement Open Main Window action: show/focus main window
- [x] 2.5 Implement Quit action: trigger app shutdown with process cleanup
- [x] 2.6 Toggle menu text between Connect/Disconnect based on process state

## 3. Status Updates

- [x] 3.1 Subscribe to ProcessEvent channel for state changes
- [x] 3.2 Update tray icon based on process state (Stopped→gray, Running→green, Error→red)
- [x] 3.3 Update menu item states dynamically when process state changes

## 4. Notifications

- [x] 4.1 Implement desktop notification on connection established
- [x] 4.2 Implement desktop notification on unexpected disconnection
- [x] 4.3 Implement desktop notification on error state
- [x] 4.4 Add notification enable/disable setting and respect it

## 5. Window Management

- [x] 5.1 Implement minimize-to-tray on window close (configurable)
- [x] 5.2 Implement restore-from-tray (show and focus main window)
- [x] 5.3 Add minimize-to-tray toggle in settings
