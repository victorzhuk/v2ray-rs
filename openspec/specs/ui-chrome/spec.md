## ADDED Requirements

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
