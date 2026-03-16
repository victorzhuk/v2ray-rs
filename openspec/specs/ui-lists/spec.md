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

### Requirement: Asynchronous node latency indicators
Node rows in the subscriptions UI SHALL display their most recent latency values and update when manual or scheduled latency refreshes complete.

#### Scenario: Manual latency result updates a row
- **WHEN** a user-triggered latency test finishes for a subscription node
- **THEN** the matching node row updates its latency text and styling without disconnecting the backend

#### Scenario: Scheduled latency result updates a row
- **WHEN** the 10-minute background refresh finishes for an enabled node
- **THEN** the matching node row updates its latency indicator and keeps the current connection state unchanged

#### Scenario: Latency color coding
- **WHEN** a node's latency is below 200ms THEN the label uses success styling (green)
- **WHEN** a node's latency is between 200ms and 499ms THEN the label uses warning styling (yellow)
- **WHEN** a node's latency is 500ms or above THEN the label uses error styling (red)
- **WHEN** a node has no latency data THEN no latency label is shown

#### Scenario: Startup latency display
- **WHEN** the app starts with a persisted latency snapshot
- **THEN** node rows immediately display their last recorded latency values

### Requirement: Manual nodes list uses boxed-list styling
The manual nodes list SHALL use `gtk::ListBox` with the `"boxed-list"` CSS class, consistent with the subscriptions list.

#### Scenario: Manual nodes list renders
- **WHEN** the `Nodes` section displays one or more manual nodes
- **THEN** the list container is a `gtk::ListBox` with boxed-list styling

### Requirement: Manual node rows use compact suffix actions
Manual node rows SHALL expose at most two visible suffix widgets, with secondary actions grouped behind a menu button.

#### Scenario: Manual node row suffixes
- **WHEN** a manual node row is displayed
- **THEN** only the enable switch and a menu button are visible as suffix widgets
