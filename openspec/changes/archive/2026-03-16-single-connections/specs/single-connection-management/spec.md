## ADDED Requirements

### Requirement: Manual proxy nodes
The system SHALL allow users to define, save, edit, enable, disable, and delete individual manual proxy nodes.

#### Scenario: Add a manual VLESS node
- **WHEN** the user clicks "Add Manual Node" and selects `VLESS`
- **THEN** the system presents a VLESS form and saves the created node to manual-node persistence

#### Scenario: Edit a manual Shadowsocks node
- **WHEN** the user edits an existing `Shadowsocks` manual node
- **THEN** the updated protocol fields are saved back to manual-node persistence

#### Scenario: Delete a manual Trojan node
- **WHEN** the user deletes a `Trojan` manual node
- **THEN** the node is removed from manual-node persistence and no longer appears in the nodes list

### Requirement: Protocol-specific manual node forms
The system SHALL provide protocol-specific forms for VLESS, VMess, Shadowsocks, and Trojan using the existing proxy-node model fields.

#### Scenario: Choose VMess in the add dialog
- **WHEN** the user selects `VMess`
- **THEN** the form shows the VMess fields required by the proxy-node model

#### Scenario: Choose Trojan in the add dialog
- **WHEN** the user selects `Trojan`
- **THEN** the form shows the Trojan fields required by the proxy-node model

### Requirement: Enabled manual nodes participate in connection planning
Enabled manual nodes SHALL participate in candidate planning and manual-node changes SHALL follow the connected-state restart policy.

#### Scenario: Enable a manual node while disconnected
- **WHEN** the user enables a manual node while the backend is stopped
- **THEN** it becomes available for candidate planning and config regeneration occurs immediately

#### Scenario: Disable a manual node while disconnected
- **WHEN** the user disables a manual node while the backend is stopped
- **THEN** it is removed from candidate planning and config regeneration occurs immediately

#### Scenario: Edit a manual node while connected
- **WHEN** the user edits a manual node while the backend is connected
- **THEN** the change is persisted but the active runtime config is marked restart-required until the user applies restart or reconnects

#### Scenario: Disable a manual node while connected
- **WHEN** the user disables a manual node while the backend is connected
- **THEN** the change is persisted but the active runtime config remains unchanged until the user applies restart, reconnects later, or discards the pending manual-node changes
