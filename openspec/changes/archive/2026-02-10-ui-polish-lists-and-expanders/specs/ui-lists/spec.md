## ADDED Requirements

### Requirement: Subscription and routing lists use boxed-list styling
List containers in subscriptions and routing pages SHALL use `gtk::ListBox` with `"boxed-list"` CSS class instead of raw `gtk::Box`.

#### Scenario: Subscription list renders with boxed-list style
- **WHEN** subscriptions page displays one or more subscriptions
- **THEN** the list container is a `gtk::ListBox` with rounded Adwaita borders

#### Scenario: Routing list renders with boxed-list style
- **WHEN** routing page displays one or more rules
- **THEN** the list container is a `gtk::ListBox` with rounded Adwaita borders

### Requirement: Subscriptions use ExpanderRow for node lists
Each subscription row SHALL use `adw::ExpanderRow` for expand/collapse of child nodes instead of manual state tracking.

#### Scenario: Subscription expands to show nodes
- **WHEN** user clicks the expander arrow on a subscription row
- **THEN** the child node rows are revealed with animation

#### Scenario: No manual expand state tracking
- **WHEN** subscription list is rendered
- **THEN** no `expanded_subs` HashSet or `ToggleExpand` message is used — expansion is handled by the widget

### Requirement: Max two visible suffix widgets per row
List rows SHALL have at most two visible suffix widgets. Secondary actions (update, delete, move up/down, edit) SHALL be placed in a `gtk::MenuButton` popover.

#### Scenario: Subscription row suffixes
- **WHEN** a subscription row is displayed
- **THEN** only the enable switch and a menu button are visible as suffixes

#### Scenario: Routing rule row suffixes
- **WHEN** a routing rule row is displayed
- **THEN** only the enable switch and a menu button are visible as suffixes

### Requirement: Wizard backend selection uses radio group
Backend CheckButtons in the wizard SHALL be linked via `set_group()` so only one backend can be selected at a time.

#### Scenario: Single backend selection
- **WHEN** user selects a different backend in the wizard
- **THEN** the previously selected backend is automatically deselected

### Requirement: Human-readable action text
Rule action labels SHALL display human-readable strings ("Proxy", "Direct", "Block") instead of Rust debug formatting.

#### Scenario: Rule action displays readable text
- **WHEN** a routing rule row is displayed
- **THEN** the action shows "Proxy", "Direct", or "Block" — not debug format like `Proxy` from `{:?}`

### Requirement: Protocol badges use accent styling
Protocol labels on subscription node rows SHALL use accent or pill styling instead of `"dim-label"`.

#### Scenario: Protocol badge is visually prominent
- **WHEN** a subscription node row displays its protocol
- **THEN** the protocol label uses accent color styling, not dim/gray text

### Requirement: No Backend Found uses StatusPage
The wizard backend detection step SHALL use `adw::StatusPage` with an error icon when no backends are found.

#### Scenario: No backends detected
- **WHEN** the wizard detects no installed backends
- **THEN** an `adw::StatusPage` displays with "dialog-error-symbolic" icon and install guidance
