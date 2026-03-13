## ADDED Requirements

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
