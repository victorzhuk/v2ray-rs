## ADDED Requirements

### Requirement: Effective DNS resolver set is derivable
The system SHALL derive, for a backend, the DNS settings, the TUN settings, and the routing rules used to generate a config, the ordered list of resolvers the generated config uses. Each entry SHALL state the resolver address, its transport, its path (`direct`, `proxy`, `routing` when the backend's routing rules decide, `system` for the operating-system resolver, or `static` for host overrides), its source (`user`, `fallback`, `bootstrap`, `system`, or `profile` when a subscription's imported profile supplied the DNS settings), and its scope (all domains or the listed domains). The list SHALL match the resolvers present in the generated config for the same inputs. Deriving it SHALL NOT change which resolvers are generated.

#### Scenario: xray TUN with DNS disabled
- **WHEN** the backend is xray, TUN is enabled, and DNS is disabled
- **THEN** the list SHALL contain `https://1.1.1.1/dns-query` and `https://8.8.8.8/dns-query` with path `proxy` and source `fallback`, and SHALL NOT contain the configured DNS servers

#### Scenario: xray with only domain-scoped servers
- **WHEN** the backend is xray, DNS is enabled, and every configured server is scoped to domains
- **THEN** the list SHALL contain the configured servers with their domain scope followed by `https://1.1.1.1/dns-query` with source `fallback` and scope all domains

#### Scenario: xray TUN bootstrap for a hostname node
- **WHEN** the backend is xray, TUN is enabled, and the node address is a hostname
- **THEN** the list SHALL start with UDP `1.1.1.1` and DoH `1.1.1.1` with path `direct`, source `bootstrap`, and scope that hostname

#### Scenario: sing-box TUN with DNS disabled
- **WHEN** the backend is sing-box, TUN is enabled, and DNS is disabled
- **THEN** the list SHALL contain DoH `1.1.1.1` with path `proxy` and source `fallback`

#### Scenario: sing-box server without detour
- **WHEN** the backend is sing-box, DNS is enabled, and a configured server has no detour
- **THEN** that entry SHALL have path `direct`

#### Scenario: No DNS section outside TUN
- **WHEN** the backend is xray or v2ray, TUN is off, DNS is disabled, and routing rules are enabled
- **THEN** the list SHALL contain one entry with path `system` and source `system`

#### Scenario: Imported profile supplies DNS
- **WHEN** the inputs come from a subscription node whose enabled imported profile carries DNS settings
- **THEN** entries built from those settings SHALL have source `profile`
