# Spec Delta: main-window

## ADDED Requirements

### Requirement: CleanupPending exposes a persistent retry control and disables Connect
While `cleanup_pending` is true, the main window SHALL disable the Connect button, suppress duplicate generic error toasts from `apply_state`, and display a persistent banner offering "Retry cleanup" until cleanup succeeds.

#### Scenario: Retry cleanup remains accessible after toast timeout
- **WHEN** `AppMsg::CleanupPending` arrives and more than 5 seconds elapse
- **THEN** a persistent banner SHALL still offer "Retry cleanup", Connect SHALL be insensitive, and only one error/cleanup notification SHALL have appeared

### Requirement: Broken settings reset flows through WorkspaceStore
`AppMsg::ResetBrokenSettings` SHALL reset `settings.toml` and update the cached `AppSettings` snapshot through `WorkspaceStore::reset_settings()` and SHALL resync `geodata_service` and `subscriptions_page` whether or not the file existed on disk.

#### Scenario: Store cache reflects reset settings immediately
- **WHEN** the user confirms resetting a broken `settings.toml`
- **THEN** `WorkspaceStore::load_settings()` SHALL immediately return `AppSettings::default()` and dependent UI/services SHALL resync
