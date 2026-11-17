# Spec Delta

## ADDED Requirements

### Requirement: Node loopback proxy client tunnels all schemes and ignores host proxy env
Egress detection requests SHALL route both HTTP and HTTPS traffic through the tested node's loopback proxy inbound and SHALL ignore ambient `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, and `NO_PROXY` environment variables. Nodes whose delay probe failed or produced no measurement SHALL skip egress detection instead of sending requests to a dead outbound.

#### Scenario: HTTPS echo endpoint tunnels through node loopback inbound
- **WHEN** a probe session requests an `https://` IP-echo endpoint for a node whose loopback inbound listens on `127.0.0.1:<port>`
- **THEN** the HTTP client SHALL connect via `http://127.0.0.1:<port>` using `CONNECT` rather than dialing the echo host directly from the host network

#### Scenario: Ambient NO_PROXY wildcard cannot bypass the node inbound
- **WHEN** the process environment sets `NO_PROXY=*` or `HTTPS_PROXY=http://127.0.0.1:9999` during a Real Delay probe
- **THEN** egress detection SHALL still tunnel exclusively through the node's own loopback inbound

#### Scenario: Failed delay node skips egress probe
- **WHEN** a tested node's Real Delay measurement fails or times out
- **THEN** the session SHALL skip the egress HTTP request for that node and leave its egress sample unknown

#### Scenario: The node proxy cannot reach a reachable endpoint
- **WHEN** egress detection targets a reachable endpoint and the node's loopback proxy refuses the request
- **THEN** the probe SHALL report no successful result and the endpoint SHALL receive no direct request
