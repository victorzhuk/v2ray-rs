# Spec Delta

## ADDED Requirements

### Requirement: Availability checks tunnel HTTPS traffic and ignore host proxy env
Availability requests for HTTP and HTTPS service URLs SHALL tunnel through the tested node's loopback proxy inbound and SHALL ignore ambient host proxy environment variables, ensuring no check exits directly from the host network interface.

#### Scenario: HTTPS service check tunnels through node inbound
- **WHEN** an availability probe checks `https://notebooklm.google.com/` or `https://gemini.google.com/about` for a node
- **THEN** every hop in the redirect chain SHALL be tunneled through that node's loopback inbound and SHALL NOT connect directly from the host

#### Scenario: Host proxy environment variables are ignored
- **WHEN** `NO_PROXY=*` or `HTTPS_PROXY` is set in the application environment during an availability probe
- **THEN** the probe client SHALL ignore those variables and route strictly through the per-node loopback inbound

#### Scenario: The node proxy cannot reach a reachable endpoint
- **WHEN** an availability check targets a reachable endpoint and the node's loopback proxy refuses the request
- **THEN** the probe SHALL report no successful result and the endpoint SHALL receive no direct request
