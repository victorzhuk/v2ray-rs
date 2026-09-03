## ADDED Requirements

### Requirement: Programmatic widget updates do not re-enter handlers
When the DNS preferences page updates its own widgets to reflect a settings change it made, those updates SHALL NOT re-enter the widgets' own change handlers, and no borrow of the shared settings state SHALL be held across a widget setter. GTK emits property notifications synchronously, so a borrow held across a setter re-enters the handler and aborts the process rather than raising a recoverable error.

#### Scenario: Applying a provider preset re-syncs the page
- **WHEN** the user applies a DNS provider preset while the IP strategy is set to something other than the preset's
- **THEN** the master enable switch and the strategy row SHALL both update to the preset's values, the settings SHALL be mutated once, and the application SHALL NOT abort

#### Scenario: Suppressed handlers do not write settings back
- **WHEN** the page drives a widget programmatically
- **THEN** the widget's change handler SHALL make no settings mutation and emit no settings change for that update

### Requirement: Detour is configurable for every backend that honors it
The detour control in the DNS server dialog SHALL retain the chosen value for every backend that can act on it — sing-box, which emits it on the server object, and xray, which expresses a direct detour as a routing rule. The value SHALL NOT be discarded on save for those backends.

#### Scenario: Detour is retained for xray
- **WHEN** the user sets a server's detour to direct while the backend is xray and saves
- **THEN** the saved server SHALL keep that detour instead of having it cleared

#### Scenario: Detour stays available for sing-box
- **WHEN** the backend is sing-box
- **THEN** the detour control SHALL behave as before
