## Purpose

Standardize main window chrome with a single header bar, page actions, tab icons, restart banner, and active node indicators.

## Requirements

### Requirement: Single HeaderBar per window
The application window SHALL have exactly one `adw::HeaderBar` at the top level. Sub-pages inside `adw::ViewStack` SHALL NOT render their own HeaderBars.

#### Scenario: Subscriptions page has no HeaderBar
- **WHEN** user navigates to the Subscriptions tab
- **THEN** the page content displays without a nested HeaderBar below the ViewSwitcher

#### Scenario: Routing page has no HeaderBar
- **WHEN** user navigates to the Routing tab
- **THEN** the page content displays without a nested HeaderBar below the ViewSwitcher

#### Scenario: Logs page has no HeaderBar
- **WHEN** user navigates to the Logs tab
- **THEN** the page content displays without a nested HeaderBar below the ViewSwitcher

### Requirement: Page action buttons in content area
Page-specific action buttons (Add Subscription, Add Rule, Presets, Clear Logs) SHALL be placed in the page content area, right-aligned at the top of the page, with `"flat"` CSS class.

#### Scenario: Subscriptions page action buttons
- **WHEN** user views the Subscriptions page
- **THEN** the "Add Subscription" button appears at the top-right of the page content

#### Scenario: Routing page action buttons
- **WHEN** user views the Routing page
- **THEN** the "Add Rule" and "Presets" buttons appear at the top-right of the page content

#### Scenario: Logs page action buttons
- **WHEN** user views the Logs page
- **THEN** the "Clear Logs" button appears at the top-right of the page content

### Requirement: ViewStack tabs have icons
Each `adw::ViewStack` page SHALL display a symbolic icon alongside its title in the ViewSwitcher.

#### Scenario: All tabs display icons
- **WHEN** the main window is visible
- **THEN** the ViewSwitcher shows icon+label for Subscriptions, Routing, Logs, and Settings tabs

### Requirement: No unnecessary widget wrappers
The wizard display in app.rs SHALL not use a redundant `gtk::Box` wrapper around the wizard widget.

#### Scenario: Wizard renders without extra wrapper
- **WHEN** the onboarding wizard is shown
- **THEN** the wizard widget is rendered directly without an intermediate Box container

### Requirement: Restart-required banner
The main window SHALL display an `adw::Banner` when the persisted runtime configuration diverges from the active runtime snapshot while the backend is starting or running.

#### Scenario: Connected DNS or routing change
- **WHEN** the backend is connected and the user changes a runtime-relevant DNS or routing setting
- **THEN** a banner appears with "Apply & Restart" and "Discard" actions

#### Scenario: Divergence resolved
- **WHEN** the user applies the restart, discards the changes, or disconnects
- **THEN** the banner is dismissed

### Requirement: Restart-required banner for manual nodes
The main window SHALL reuse the restart-required banner for connected manual-node changes that diverge from the launched runtime snapshot.

#### Scenario: Connected manual-node change
- **WHEN** the backend is connected and the user adds, edits, deletes, or toggles the enabled state of a manual node
- **THEN** a banner appears with `Apply & Restart` and `Discard` actions

#### Scenario: Discard connected manual-node change
- **WHEN** the user selects `Discard` after connected manual-node changes
- **THEN** the banner is dismissed and the persisted manual-node set returns to the launched snapshot

### Requirement: Live subscription editing while connected
Subscription and node controls (toggles, reorder, add, menu actions) SHALL remain enabled while the backend is starting or running; connected changes SHALL be persisted without interrupting the running session and SHALL reuse the restart-required banner.

#### Scenario: Toggle a subscription node while connected
- **WHEN** the backend is connected and the user toggles, reorders, adds, or removes subscription nodes or subscriptions
- **THEN** the change is persisted, the running connection stays up, and the restart-required banner offers `Apply & Restart`

### Requirement: Active node indicator
The subscription and manual node lists SHALL mark the currently connected node while the backend is running.

#### Scenario: Connected node marked in the list
- **WHEN** the backend reaches Running with a resolved node
- **THEN** the corresponding row shows a `Connected` tag, which is removed when the session ends or the connection switches nodes
