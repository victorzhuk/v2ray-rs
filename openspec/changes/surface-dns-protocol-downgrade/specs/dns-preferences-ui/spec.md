## ADDED Requirements

### Requirement: Downgrade warning in the DNS server dialog
The DNS server dialog SHALL keep every protocol selectable regardless of the active backend, and SHALL show an inline warning when the selected protocol will be downgraded for the active backend, naming the effective protocol. The warning SHALL be derived from the same core compatibility function used by the config generators.

#### Scenario: Selecting an unsupported protocol warns inline
- **WHEN** the active backend is v2ray and the user selects DoT, DoQ, or H3 (or the backend is xray and the user selects H3) in the server dialog
- **THEN** an inline warning SHALL state that the server will run as DoH on the active backend, and saving SHALL still be allowed

#### Scenario: Supported protocol shows no warning
- **WHEN** the selected protocol is natively supported by the active backend
- **THEN** no downgrade warning SHALL be shown

### Requirement: Downgrade indicator on saved DNS server rows
The DNS server list SHALL passively mark saved servers whose configured protocol the active backend will not honor, so a backend switch cannot leave a silent downgrade invisible.

#### Scenario: Backend switch reveals affected servers
- **WHEN** the active backend changes and a saved DNS server's protocol will be downgraded on it
- **THEN** that server's row SHALL show a downgrade indicator naming the effective protocol

#### Scenario: Stored protocol is preserved
- **WHEN** a saved server's protocol is unsupported by the active backend
- **THEN** the stored protocol value SHALL remain unchanged unless the user edits it
