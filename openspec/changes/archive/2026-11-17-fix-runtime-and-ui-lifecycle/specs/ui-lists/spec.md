# Spec Delta

## ADDED Requirements

### Requirement: Editor dialogs stay open on validation error
Add and edit dialogs for routing rules, manual nodes, and subscriptions SHALL validate inputs before closing so invalid submissions display inline errors and block confirmation without discarding user input.

#### Scenario: Invalid routing rule or manual node edit preserves typed input
- **WHEN** the user submits an invalid CIDR, domain pattern, or port in an Add/Edit dialog
- **THEN** the dialog SHALL remain open with inline validation feedback and all user-typed fields intact

### Requirement: Node widgets update by ID and wizard preserves loaded state
`SubscriptionsPage` SHALL update per-node latency and egress widgets via an explicit `NodeId` map rather than walking `AdwExpanderRow` internal children, and `OnboardingWizard` SHALL preserve loaded `AppSettings`, strip `v2raytun://import/` URL prefixes, and sync `NodesPage` on completion.

#### Scenario: Subscription node latency badge updates in place via widget map
- **WHEN** a TCP ping or Real Delay result arrives for an expanded subscription node
- **THEN** `SubscriptionsPage` SHALL update that node's latency and egress labels directly via its `NodeId` widget map without rebuilding the list or walking internal GTK container children

#### Scenario: Wizard strips v2raytun import prefix and preserves settings
- **WHEN** the user pastes `v2raytun://import/https://example.com/sub` in `OnboardingWizard` and finishes setup
- **THEN** the wizard SHALL import `https://example.com/sub`, retain pre-existing `AppSettings` fields, and sync `NodesPage`
