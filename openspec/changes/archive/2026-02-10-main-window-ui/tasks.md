# Tasks: Main Window UI

## 1. Application Scaffolding

- [x] 1.1 Add relm4, gtk4, libadwaita dependencies to Cargo.toml
- [x] 1.2 Create AdwApplication entry point with main window
- [x] 1.3 Implement AdwApplicationWindow with header bar and AdwViewStack tab navigation
- [x] 1.4 Define root AppModel and AppMsg for Relm4 message passing

## 2. Connection Status Bar

- [x] 2.1 Implement bottom status bar widget showing connection state text
- [x] 2.2 Add Connect/Disconnect button to status bar
- [x] 2.3 Subscribe to process state events and update status bar dynamically

## 3. Subscriptions Page

- [x] 3.1 Create Subscriptions page component with list view
- [x] 3.2 Implement subscription list item (name, URL, node count, last updated)
- [x] 3.3 Implement "Add Subscription" dialog (name + URL input)
- [x] 3.4 Implement expandable node list within subscription (name, address, protocol, enable toggle)
- [x] 3.5 Implement subscription delete with confirmation dialog
- [x] 3.6 Implement manual subscription update button

## 4. Routing Rules Page

- [x] 4.1 Create Routing page component with ordered list view
- [x] 4.2 Implement rule list item (match condition, action, drag handle)
- [x] 4.3 Implement "Add Rule" dialog (rule type selector, match value input, action selector)
- [x] 4.4 Implement drag-and-drop rule reordering
- [x] 4.5 Implement rule edit and delete actions
- [x] 4.6 Add preset rules button with predefined templates

## 5. Logs Page

- [x] 5.1 Create Logs page component with scrollable text view
- [x] 5.2 Implement live log streaming from process manager's log buffer
- [x] 5.3 Implement auto-scroll to latest log entry
- [x] 5.4 Show "Process not running" indicator when backend is stopped

## 6. Settings Page

- [x] 6.1 Create Settings page component with sections
- [x] 6.2 Implement backend selection section (radio buttons for detected backends)
- [x] 6.3 Implement proxy port configuration (SOCKS5, HTTP port inputs)
- [x] 6.4 Implement auto-update interval configuration
- [x] 6.5 Implement language selection dropdown (English, Russian)
- [x] 6.6 Implement minimize-to-tray toggle
- [x] 6.7 Implement notification toggle
- [x] 6.8 Save settings on change and trigger relevant regeneration

## 7. Onboarding Wizard

- [x] 7.1 Create wizard component using AdwCarousel
- [x] 7.2 Implement Welcome step
- [x] 7.3 Implement Backend Detection step (show found backends, select one)
- [x] 7.4 Implement Subscription Import step (URL input + skip option)
- [x] 7.5 Implement Completion step (save settings, mark onboarding done)
- [x] 7.6 Show wizard on first launch, skip on subsequent launches

## 8. Internationalization

- [x] 8.1 Set up gettext-rs with locale/ directory structure
- [x] 8.2 Extract translatable strings with xgettext markers
- [x] 8.3 Create en_US.po base translation
- [x] 8.4 Create ru_RU.po Russian translation
- [x] 8.5 Implement runtime language switching
