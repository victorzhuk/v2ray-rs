# Spec Delta

## ADDED Requirements

### Requirement: DNS dialog preserves unset ports, deletes exact host pairs, and refreshes Effective DNS
Opening the DNS server editor on a server with `port = None` SHALL keep `port = None` on save unless the user customizes the port, changing protocol SHALL update an uncustomized port field to the new protocol's default, deleting a static `HostOverride` SHALL remove only the matching `(domain, ip)` entry, and `Effective DNS` SHALL refresh when routing rules change in auto-derived mode.

#### Scenario: Editing server tag without touching port preserves default None
- **WHEN** the user edits a DoH server whose stored `port` is `None`, changes only its tag, and saves
- **THEN** the saved server SHALL retain `port = None` rather than persisting `port = Some(1)` or `Some(443)`

#### Scenario: Deleting one IP of a multi-address host override keeps siblings
- **WHEN** `dns.hosts` contains `("api.example.com", "1.2.3.4")` and `("api.example.com", "2001:db8::1")` and the user deletes the `1.2.3.4` row
- **THEN** only `("api.example.com", "1.2.3.4")` SHALL be removed and `("api.example.com", "2001:db8::1")` SHALL remain
