## MODIFIED Requirements

### Requirement: Connect button has icon and label
The connect/disconnect button SHALL display both a symbolic icon and a text label.

#### Scenario: Disconnected state button appearance
- **WHEN** the proxy is disconnected
- **THEN** the button shows `network-wireless-symbolic` icon with "Connect" label and `"suggested-action"` styling

#### Scenario: Connected state button appearance
- **WHEN** the proxy is connected
- **THEN** the button shows `network-wireless-disabled-symbolic` icon with "Disconnect" label and `"destructive-action"` styling

#### Scenario: Starting state button is actionable
- **WHEN** the connection is starting, including an in-place crash respawn
- **THEN** the button shows the "Disconnect" label with `"destructive-action"` styling, stays sensitive, and invoking it cancels the attempt
