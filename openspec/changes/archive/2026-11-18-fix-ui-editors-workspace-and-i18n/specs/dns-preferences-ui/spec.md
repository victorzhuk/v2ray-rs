# Spec Delta

## MODIFIED Requirements

### Requirement: Effective DNS summary
The DNS page SHALL show a read-only "Effective DNS" summary of the resolvers a connection with the current settings, selected backend, TUN state, and enabled subscription and manual node addresses would use, listing each resolver's address, transport, path, source, and scope. The summary SHALL stay visible and readable while the DNS master toggle is off. It SHALL update when DNS, TUN, backend, routing, or node settings change. When one or more enabled subscriptions have an enabled imported profile with DNS settings, the summary SHALL name those subscriptions and state that their nodes use the profile's DNS instead.

#### Scenario: Fallback shown while DNS is off
- **WHEN** the backend is xray, TUN is enabled, DNS is disabled, and the user opens the DNS page
- **THEN** the summary SHALL list `https://1.1.1.1/dns-query` and `https://8.8.8.8/dns-query` as fallback resolvers reached through the proxy

#### Scenario: Hostname node bootstrap resolvers shown in Effective DNS
- **WHEN** the backend is xray, TUN is enabled, and an enabled subscription or manual node is addressed by hostname `edge.example.com`
- **THEN** the Effective DNS summary SHALL include the `bootstrap` resolvers (`1.1.1.1:53` UDP and `https://1.1.1.1/dns-query` DoH) scoped to `edge.example.com`

#### Scenario: Summary follows edits
- **WHEN** the user enables DNS on that page
- **THEN** the summary SHALL replace the fallback entries with the configured servers without reopening Preferences

#### Scenario: Imported profile named
- **WHEN** an enabled subscription named "Provider" has an enabled imported profile with DNS settings
- **THEN** the summary SHALL state that nodes from "Provider" use that profile's DNS

### Requirement: DNS dialog preserves unset ports, deletes exact host pairs, and refreshes Effective DNS
Opening the DNS server editor on a server with `port = None` SHALL initialize the port control to `protocol.default_port()` and keep `port = None` on save unless the user customizes the port; changing protocol SHALL update an uncustomized port field to the new protocol's default; opening a server with `detour = None` while `sing-box` is active SHALL initialize the detour selector to `"direct"` matching `sing-box` runtime behavior; static `HostOverride` rows SHALL support editing and deleting exact `(domain, ip)` pairs while rejecting duplicate `(domain, ip)` entries; and `Effective DNS` SHALL refresh when routing rules change in auto-derived mode.

#### Scenario: Editing server tag without touching port preserves default None
- **WHEN** the user opens the edit dialog for a DoH server whose stored `port` is `None`, changes only its tag, and saves
- **THEN** the port control SHALL display `443` on open and the saved server SHALL retain `port = None` rather than persisting `port = Some(1)` or `Some(443)`

#### Scenario: Editing server with unset detour on sing-box defaults selector to direct
- **WHEN** the active backend is `sing-box` and the user opens the edit dialog for a server whose stored `detour` is `None`
- **THEN** the detour selector SHALL initialize to `direct` rather than `proxy`

#### Scenario: Deleting one IP of a multi-address host override keeps siblings
- **WHEN** `dns.hosts` contains `("api.example.com", "1.2.3.4")` and `("api.example.com", "2001:db8::1")` and the user deletes the `1.2.3.4` row
- **THEN** only `("api.example.com", "1.2.3.4")` SHALL be removed and `("api.example.com", "2001:db8::1")` SHALL remain

#### Scenario: Editing one IP of a multi-address host override keeps siblings and rejects duplicates
- **WHEN** `dns.hosts` contains `("api.example.com", "1.2.3.4")` and `("api.example.com", "2001:db8::1")` and the user edits `1.2.3.4` to `1.2.3.5`
- **THEN** only `("api.example.com", "1.2.3.4")` SHALL become `("api.example.com", "1.2.3.5")`, `("api.example.com", "2001:db8::1")` SHALL remain intact, and attempting to edit it to `2001:db8::1` SHALL be rejected inline
