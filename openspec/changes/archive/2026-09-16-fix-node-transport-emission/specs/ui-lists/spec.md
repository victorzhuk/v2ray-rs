## ADDED Requirements

### Requirement: Node editor marks fields sing-box ignores
The manual node editor SHALL label the gRPC multi mode switch and the REALITY spider X entry as not used by sing-box, regardless of the active backend, because a node's settings outlive a backend switch. The fields SHALL stay editable and their values SHALL be saved as before.

#### Scenario: gRPC multi mode label
- **WHEN** the user edits a node with the gRPC transport
- **THEN** the multi mode row SHALL state that sing-box does not use it

#### Scenario: Spider X label
- **WHEN** the user edits a node with REALITY enabled
- **THEN** the spider X row SHALL state that sing-box does not use it, and an entered value SHALL still be saved
