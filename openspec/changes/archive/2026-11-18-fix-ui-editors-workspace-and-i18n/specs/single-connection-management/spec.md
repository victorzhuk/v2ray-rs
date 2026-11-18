# Spec Delta

## MODIFIED Requirements

### Requirement: Protocol-specific manual node forms
The system SHALL provide protocol-specific forms for VLESS, VMess, Shadowsocks, and Trojan using the existing proxy-node model fields, and editing an existing manual node SHALL preserve transport metadata not exposed as editable form fields (such as custom WebSocket HTTP headers) when the transport type remains WebSocket.

#### Scenario: Choose VMess in the add dialog
- **WHEN** the user selects `VMess`
- **THEN** the form shows the VMess fields required by the proxy-node model

#### Scenario: Choose Trojan in the add dialog
- **WHEN** the user selects `Trojan`
- **THEN** the form shows the Trojan fields required by the proxy-node model

#### Scenario: Editing a WebSocket manual node preserves custom headers
- **WHEN** an existing manual node has `TransportSettings::Ws` with custom `headers` (for example `{"User-Agent": "custom"}`) and the user edits its remark, address, or WebSocket path/host and saves
- **THEN** the saved manual node SHALL retain its existing custom `headers` map instead of clearing it
