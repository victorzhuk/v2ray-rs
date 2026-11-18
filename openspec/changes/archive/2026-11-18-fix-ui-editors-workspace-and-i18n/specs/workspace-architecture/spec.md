# Spec Delta

## MODIFIED Requirements

### Requirement: Authoritative WorkspaceStore owns state snapshots and async persistence
`WorkspaceStore` SHALL hold authoritative `Arc`-shared snapshots of `AppSettings`, subscriptions, manual nodes, routing rules, and latency metadata and serialize all disk writes and corrupt-store resets (including resetting unreadable `routing_rules.json` to an empty rule set) off the GTK main thread, eliminating direct unlocked filesystem mutations in UI components.

#### Scenario: Concurrent UI mutations serialize through WorkspaceStore without clobbering
- **WHEN** Preferences edits `AppSettings` while a background subscription refresh or wizard completion updates subscriptions and manual nodes
- **THEN** all mutations SHALL flow through `WorkspaceStore` in order, broadcast updated `Arc` snapshots to subscribers, and persist atomically off the GTK main thread without stale-file overwrites

#### Scenario: Corrupt routing rules reset flows through WorkspaceStore
- **WHEN** `routing_rules.json` fails to load on startup and the user confirms "Reset to Empty" on the routing error page
- **THEN** `WorkspaceStore` SHALL atomically persist an empty `RoutingRuleSet` on its serialized worker thread, clear the stored routing load error, and broadcast the empty snapshot without direct `std::fs::remove_file` calls on the GTK thread
