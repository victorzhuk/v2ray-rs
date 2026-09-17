## ADDED Requirements

### Requirement: DNS hijack mode effect is explained
The TUN page SHALL explain the DNS hijack modes next to the selector: `hijack` routes DNS queries captured by the tunnel to the backend's resolver, while `native` and `disabled` both leave DNS queries to be carried as ordinary traffic without being answered by the backend's resolver.

#### Scenario: Hijack row note
- **WHEN** the user opens the TUN page with sing-box or xray selected
- **THEN** the DNS hijack row SHALL state that `native` and `disabled` currently behave the same and do not hand DNS queries to the backend's resolver
